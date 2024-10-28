use crate::{prelude::*, AllDevices};
use alloc::sync::Arc;
use axhal::mem::phys_to_virt;
use pcie::{preludes::*, PciDevice};

impl AllDevices {
    pub(crate) fn probe_bus_devices(&mut self) {
        let base_vaddr = phys_to_virt(axconfig::PCI_ECAM_BASE.into());

        info!("Init PCIE @{:#X}", axconfig::PCI_ECAM_BASE);

        let mut root = pcie::RootGeneric::new(base_vaddr.as_usize());

        root.enumerate().for_each(|device| {
            let address = device.address();
            debug!("PCI {}", device);

            if let PciDevice::Endpoint(mut ep) = device {
                ep.update_command(|cmd| {
                    cmd | CommandRegister::IO_ENABLE
                        | CommandRegister::MEMORY_ENABLE
                        | CommandRegister::BUS_MASTER_ENABLE
                });

                let ep = Arc::new(ep);

                for_each_drivers!(type Driver, {
                    let ep = ep.clone();
                    if let Some(dev) = Driver::probe_pcie(&mut root, ep) {
                        info!(
                            "registered a new {:?} device at {}: {:?}",
                            dev.device_type(),
                            address,
                            dev.device_name(),
                        );
                        self.add_device(dev);
                    }
                });
            }
        });
    }
}
