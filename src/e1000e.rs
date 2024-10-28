use core::{alloc::Layout, mem, ptr::NonNull};

use alloc::{boxed::Box, collections::vec_deque::VecDeque, sync::Arc, vec::Vec};

use axalloc::global_allocator;

use axhal::mem::{phys_to_virt, PhysAddr};
use driver_net::{
    BaseDriverOps, DevError, DevResult, DeviceType, EthernetAddress, NetBufPtr, NetDriverOps,
};
use e1000_driver::e1000::{
    register_kernel, DMAInfo, E1000XmitConfig, KernelFunc, MacAddress, NetDevSettings, Settings,
    E1000,
};
use kspin::SpinNoIrq;
use pcie::preludes::*;

const QS: usize = 64;

pub struct E1000E {
    inner: SpinNoIrq<E1000>,
    mac: MacAddress,
    rx_buffer_queue: VecDeque<NetBufPtr>,
}

unsafe impl Send for E1000E {}
unsafe impl Sync for E1000E {}

impl E1000E {
    pub fn new<C: Chip>(ep: Arc<Endpoint<C>>) -> Self {
        let (_, device_id) = ep.id();
        let settings = Settings {
            enable_msi: true,
            mtu: 1500,
        };
        let (pin, line) = ep.interrupt();
        info!("pin {pin} line {line}");

        register_kernel(KFun { pcie: ep });

        let mut e1000 = E1000::new(device_id as _, settings).unwrap();
        let mut mac = e1000.read_mac_addr_generic();

        let net_dev_settings = NetDevSettings {
            iff_promisc: false,
            iff_allmulti: false,
            mc_list: mac.0.as_mut_ptr(),
            mc_list_len: 6,
            uc_list: mac.0.as_mut_ptr(),
            uc_list_len: 6,
        };

        let settings = net_dev_settings;
        e1000.open(settings).unwrap();
        let rx_buffer_queue = VecDeque::with_capacity(QS);

        Self {
            inner: SpinNoIrq::new(e1000),
            mac,
            rx_buffer_queue,
        }
    }
}

impl BaseDriverOps for E1000E {
    fn device_name(&self) -> &str {
        "E1000 "
    }

    fn device_type(&self) -> DeviceType {
        DeviceType::Net
    }
}

impl NetDriverOps for E1000E {
    fn mac_address(&self) -> EthernetAddress {
        let mac = self.mac;
        EthernetAddress(mac.0)
    }

    fn can_transmit(&self) -> bool {
        let mut e1000 = self.inner.lock();
        if e1000.is_link_up() {
            return true;
        }
        let _ = e1000.irq_handle(1);
        e1000.is_link_up()
    }

    fn can_receive(&self) -> bool {
        let mut e1000 = self.inner.lock();
        if e1000.is_link_up() {
            return true;
        }
        let _ = e1000.irq_handle(1);
        e1000.is_link_up()
    }

    fn rx_queue_size(&self) -> usize {
        QS
    }

    fn tx_queue_size(&self) -> usize {
        QS
    }

    fn recycle_rx_buffer(&mut self, rx_buf: NetBufPtr) -> DevResult {
        unsafe {
            drop(Box::from_raw(rx_buf.raw_ptr::<Vec<u8>>()));
        }
        Ok(())
    }

    fn recycle_tx_buffers(&mut self) -> DevResult {
        Ok(())
    }

    fn transmit(&mut self, mut tx_buf: NetBufPtr) -> DevResult {
        let r = self
            .inner
            .lock()
            .xmit(
                E1000XmitConfig {
                    timestamp: 0,
                    segs: 1,
                    ipv4: true,
                    no_fcs: true,
                    vlan_tag_present: false,
                },
                tx_buf.packet_mut(),
            )
            .inspect_err(|e| warn!("xmit {}", e))
            .map_err(|_e| DevError::Again);

        unsafe {
            drop(Box::from_raw(tx_buf.raw_ptr::<Vec<u8>>()));
        }
        r?;
        Ok(())
    }

    fn receive(&mut self) -> DevResult<NetBufPtr> {
        if !self.rx_buffer_queue.is_empty() {
            // RX buffer have received packets.
            Ok(self.rx_buffer_queue.pop_front().unwrap())
        } else {
            let mut e1000 = self.inner.lock();
            e1000.clean_tx_irq();
            let pks = e1000.clean_rx_irq(64);
            if !pks.is_empty() {
                for packet in pks {
                    let src = packet.data.to_vec();
                    let size = src.len();
                    let mut tx_buf = Box::new(src);
                    let tx_buf_ptr = tx_buf.as_mut_ptr();

                    self.rx_buffer_queue.push_back(NetBufPtr::new(
                        NonNull::new(Box::into_raw(tx_buf) as *mut u8).unwrap(),
                        NonNull::new(tx_buf_ptr).unwrap(),
                        size,
                    ));
                }
                Ok(self.rx_buffer_queue.pop_front().unwrap())
            } else {
                Err(DevError::Again)
            }
        }
    }

    fn alloc_tx_buffer(&mut self, size: usize) -> DevResult<NetBufPtr> {
        let mut tx_buf = Box::new(alloc::vec![0; size]);
        let tx_buf_ptr = tx_buf.as_mut_ptr();

        Ok(NetBufPtr::new(
            NonNull::new(Box::into_raw(tx_buf) as *mut u8).unwrap(),
            NonNull::new(tx_buf_ptr).unwrap(),
            size,
        ))
    }
}

struct KFun<C: Chip> {
    pcie: Arc<Endpoint<C>>,
}

impl<C: Chip> KernelFunc for KFun<C> {
    fn bar_n_remap(&self, bar: usize) -> usize {
        let bar = self.pcie.bar(bar as _).unwrap();

        let addr = match bar {
            Bar::Memory32 {
                address,
                size: _,
                prefetchable: _,
            } => address as usize,
            Bar::Memory64 {
                address,
                size: _,
                prefetchable: _,
            } => address as usize,
            Bar::Io { port: _ } => todo!(),
        };

        phys_to_virt(PhysAddr::from(addr)).as_usize()
    }

    fn delay(&self, duration: core::time::Duration) {
        axhal::time::busy_wait(duration)
    }

    fn pci_read_config_word(&self, offset: i32) -> u16 {
        debug!("pci read config offset {}", offset);
        let dword = unsafe { self.pcie.read(self.pcie.address(), offset as _) };
        let ptr = &dword as *const u32 as usize as *const u16;
        unsafe { *ptr }
    }

    fn pci_cap_offset(&self) -> i32 {
        0
    }

    fn dma_alloc_coherent(&self, size: usize) -> DMAInfo {
        let dma =
            unsafe { global_allocator().alloc(Layout::from_size_align_unchecked(size, size)) }
                .unwrap();
        DMAInfo {
            dma_addr: dma.as_ptr() as _,
            cpu_addr: dma.as_ptr() as usize,
            size,
        }
    }

    fn dma_free_coherent(&self, dma: DMAInfo) {
        unsafe {
            global_allocator().dealloc(
                NonNull::new_unchecked(dma.cpu_addr as *mut u8),
                Layout::from_size_align_unchecked(dma.size, dma.size),
            );
        }
    }

    fn enable_net(&self) {}

    fn on_xmit_completed(&self, pkts: u32, bytes: u32) {}

    fn iounmap(&self, addr: usize) {}
}
