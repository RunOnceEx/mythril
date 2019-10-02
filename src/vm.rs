use crate::error::{Error, Result};
use crate::memory::GuestPhysAddr;
use crate::registers::{IdtrBase, GdtrBase, Cr4};
use crate::vmcs;
use crate::vmx;
use alloc::vec::Vec;
use x86_64::registers::rflags;
use x86_64::registers::rflags::RFlags;
use x86_64::registers::model_specific::{Msr, FsBase, GsBase};
use x86_64::registers::control::Cr0;
use x86_64::structures::paging::page::Size4KiB;
use x86_64::structures::paging::FrameAllocator;
use x86_64::PhysAddr;

pub struct VirtualMachineConfig {
    images: Vec<(Vec<u8>, GuestPhysAddr)>,
    memory: u64, // number of 4k pages
}

impl VirtualMachineConfig {
    pub fn new(start_addr: GuestPhysAddr, memory: u64) -> VirtualMachineConfig {
        VirtualMachineConfig {
            images: vec![],
            memory: memory,
        }
    }

    pub fn load_image(&mut self, image: Vec<u8>, addr: GuestPhysAddr) -> Result<()> {
        self.images.push((image, addr));
        Ok(())
    }
}

pub struct VirtualMachine {
    vmcs: vmcs::Vmcs,
    config: VirtualMachineConfig,
}

impl VirtualMachine {
    pub fn new(
        vmx: &mut vmx::Vmx,
        alloc: &mut impl FrameAllocator<Size4KiB>,
        config: VirtualMachineConfig,
    ) -> Result<Self> {
        let mut vmcs = vmcs::Vmcs::new(alloc)?.activate(vmx)?;

        //TODO: initialize the vmcs from the config
        Self::initialize_host_vmcs(&mut vmcs);

        let vmcs = vmcs.deactivate();

        Ok(Self {
            vmcs: vmcs,
            config: config
        })
    }

    fn initialize_host_vmcs(vmcs: &mut vmcs::ActiveVmcs) -> Result<()> {
        const IA32_VMX_CR0_FIXED0_MSR: u32 = 0x486;
        const IA32_VMX_CR4_FIXED0_MSR: u32 = 0x488;
        let cr0_fixed = Msr::new(IA32_VMX_CR0_FIXED0_MSR);
        let cr4_fixed = Msr::new(IA32_VMX_CR4_FIXED0_MSR);

        let (new_cr0, new_cr4) = unsafe {
            (
                cr0_fixed.read() | Cr0::read().bits(),
                cr4_fixed.read() | Cr4::read(),
            )
        };

        vmcs.write_field(vmcs::VmcsField::HOST_CR0, new_cr0)?;
        vmcs.write_field(vmcs::VmcsField::HOST_CR4, new_cr4)?;

        vmcs.write_field(vmcs::VmcsField::HOST_ES_SELECTOR, 0x10)?;
        vmcs.write_field(vmcs::VmcsField::HOST_CS_SELECTOR, 0x08)?;
        vmcs.write_field(vmcs::VmcsField::HOST_SS_SELECTOR, 0x10)?;
        vmcs.write_field(vmcs::VmcsField::HOST_DS_SELECTOR, 0x10)?;
        vmcs.write_field(vmcs::VmcsField::HOST_FS_SELECTOR, 0x10)?;

        vmcs.write_field(vmcs::VmcsField::HOST_GS_SELECTOR, 0x10)?;
        vmcs.write_field(vmcs::VmcsField::HOST_TR_SELECTOR, 0x28)?;

        vmcs.write_field(vmcs::VmcsField::HOST_IA32_SYSENTER_CS, 0x00)?;
        vmcs.write_field(vmcs::VmcsField::HOST_IA32_SYSENTER_ESP, 0x00)?;
        vmcs.write_field(vmcs::VmcsField::HOST_IA32_SYSENTER_EIP, 0x00)?;

        vmcs.write_field(vmcs::VmcsField::HOST_IDTR_BASE, IdtrBase::read().as_u64())?;
        vmcs.write_field(vmcs::VmcsField::HOST_GDTR_BASE, GdtrBase::read().as_u64())?;

        vmcs.write_field(vmcs::VmcsField::HOST_FS_BASE, FsBase::read().as_u64())?;
        vmcs.write_field(vmcs::VmcsField::HOST_GS_BASE, GsBase::read().as_u64())?;

        Ok(())
    }

    pub fn launch(self, vmx: &mut vmx::Vmx) -> Result<VirtualMachineRunning> {
        let rflags = unsafe {
            let rflags: u64;
            asm!("vmlaunch; pushfq; popq $0"
                 : "=r"(rflags)
                 :: "rflags"
                 : "volatile");
            rflags
        };

        let rflags = rflags::RFlags::from_bits_truncate(rflags);

        if rflags.contains(RFlags::CARRY_FLAG) {
            return Err(Error::VmFailInvalid);
        } else if rflags.contains(RFlags::ZERO_FLAG) {
            return Err(Error::VmFailValid);
        }

        Ok(VirtualMachineRunning {
            vmcs: self.vmcs.activate(vmx)?,
        })
    }
}

pub struct VirtualMachineRunning<'a> {
    vmcs: vmcs::ActiveVmcs<'a>,
}