//! Defines types and probe methods of all supported devices.

#![allow(unused_imports, dead_code)]

use core::ptr::NonNull;

use crate::AxDeviceEnum;
use driver_common::DeviceType;

#[cfg(feature = "virtio")]
use crate::virtio::{self, VirtIoDevMeta};

#[cfg(feature = "bus-pci")]
use driver_pci::{DeviceFunction, DeviceFunctionInfo, PciRoot};
#[cfg(feature = "bus-pci")]
use pcie::{Chip, PciDevice, RootComplex};

pub use super::dummy::*;

pub trait DriverProbe {
    fn probe_global() -> Option<AxDeviceEnum> {
        None
    }

    #[cfg(bus = "mmio")]
    fn probe_mmio(_mmio_base: usize, _mmio_size: usize) -> Option<AxDeviceEnum> {
        None
    }

    #[cfg(bus = "pci")]
    fn probe_pci(
        _root: &mut PciRoot,
        _bdf: DeviceFunction,
        _dev_info: &DeviceFunctionInfo,
    ) -> Option<AxDeviceEnum> {
        None
    }

    #[cfg(bus = "pci")]
    fn probe_pcie<C: Chip>(
        _root: &mut RootComplex<C>,
        _ep: Arc<pcie::Endpoint<C>>,
    ) -> Option<AxDeviceEnum> {
        use pcie::{Chip, RootComplex};

        None
    }
}

#[cfg(net_dev = "virtio-net")]
register_net_driver!(
    <virtio::VirtIoNet as VirtIoDevMeta>::Driver,
    <virtio::VirtIoNet as VirtIoDevMeta>::Device
);

#[cfg(block_dev = "virtio-blk")]
register_block_driver!(
    <virtio::VirtIoBlk as VirtIoDevMeta>::Driver,
    <virtio::VirtIoBlk as VirtIoDevMeta>::Device
);

#[cfg(display_dev = "virtio-gpu")]
register_display_driver!(
    <virtio::VirtIoGpu as VirtIoDevMeta>::Driver,
    <virtio::VirtIoGpu as VirtIoDevMeta>::Device
);

cfg_if::cfg_if! {
    if #[cfg(block_dev = "ramdisk")] {
        pub struct RamDiskDriver;
        register_block_driver!(RamDiskDriver, driver_block::ramdisk::RamDisk);

        impl DriverProbe for RamDiskDriver {
            fn probe_global() -> Option<AxDeviceEnum> {
                // TODO: format RAM disk
                Some(AxDeviceEnum::from_block(
                    driver_block::ramdisk::RamDisk::new(0x100_0000), // 16 MiB
                ))
            }
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(block_dev = "bcm2835-sdhci")]{
        pub struct BcmSdhciDriver;
        register_block_driver!(MmckDriver, driver_block::bcm2835sdhci::SDHCIDriver);

        impl DriverProbe for BcmSdhciDriver {
            fn probe_global() -> Option<AxDeviceEnum> {
                debug!("mmc probe");
                driver_block::bcm2835sdhci::SDHCIDriver::try_new().ok().map(AxDeviceEnum::from_block)
            }
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(net_dev = "ixgbe")] {
        use crate::ixgbe::IxgbeHalImpl;
        use axhal::mem::phys_to_virt;
        pub struct IxgbeDriver;
        register_net_driver!(IxgbeDriver, driver_net::ixgbe::IxgbeNic<IxgbeHalImpl, 1024, 1>);
        impl DriverProbe for IxgbeDriver {
            #[cfg(bus = "pci")]
            fn probe_pci(
                    root: &mut driver_pci::PciRoot,
                    bdf: driver_pci::DeviceFunction,
                    dev_info: &driver_pci::DeviceFunctionInfo,
                ) -> Option<crate::AxDeviceEnum> {
                    use driver_net::ixgbe::{INTEL_82599, INTEL_VEND, IxgbeNic};
                    if dev_info.vendor_id == INTEL_VEND && dev_info.device_id == INTEL_82599 {
                        // Intel 10Gb Network
                        info!("ixgbe PCI device found at {:?}", bdf);

                        // Initialize the device
                        // These can be changed according to the requirments specified in the ixgbe init function.
                        const QN: u16 = 1;
                        const QS: usize = 1024;
                        let bar_info = root.bar_info(bdf, 0).unwrap();
                        match bar_info {
                            driver_pci::BarInfo::Memory {
                                address,
                                size,
                                ..
                            } => {
                                let ixgbe_nic = IxgbeNic::<IxgbeHalImpl, QS, QN>::init(
                                    phys_to_virt((address as usize).into()).into(),
                                    size as usize
                                )
                                .expect("failed to initialize ixgbe device");
                                return Some(AxDeviceEnum::from_net(ixgbe_nic));
                            }
                            driver_pci::BarInfo::IO { .. } => {
                                error!("ixgbe: BAR0 is of I/O type");
                                return None;
                            }
                        }
                    }
                    None
            }
        }
    }
}

cfg_if::cfg_if! {
if #[cfg(net_dev = "e1000")] {
use axhal::mem::phys_to_virt;
use alloc::sync::Arc;
use crate::e1000e::E1000E;
        use pcie::preludes::*;
pub struct E1000Driver;
register_net_driver!(E1000Driver, E1000E);
impl DriverProbe for E1000Driver {
    #[cfg(bus = "pci")]
    fn probe_pci(
        root: &mut PciRoot,
        bdf: DeviceFunction,
        dev_info: &DeviceFunctionInfo,
    ) -> Option<crate::AxDeviceEnum> {
        info!("check e1000");
        None
    }
    #[cfg(bus = "pci")]
    fn probe_pcie<C: Chip>(
        _root: &mut RootComplex<C>,
        dev: Arc< pcie::Endpoint<C>>,
    ) -> Option<AxDeviceEnum> {
        let (vid, did) = dev.id();
        let device_id_list = [0x10D3, 0x0DC8];

        if vid == 0x8086 && device_id_list.contains( &did) {
            info!("E1000E PCI device found at {:?}", dev.address());
            // Initialize the device
            // These can be changed according to the requirments specified in the ixgbe init function.
            let e1000 = E1000E::new(dev);
            return Some(AxDeviceEnum::from_net(e1000));
        }

        None
    }
}
}
}
