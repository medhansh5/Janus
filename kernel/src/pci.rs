use core::arch::asm;

const CONFIG_ADDRESS: u16 = 0xCF8;
const CONFIG_DATA: u16 = 0xCFC;

pub unsafe fn outl(port: u16, val: u32) {
    asm!("out dx, eax", in("dx") port, in("eax") val, options(nomem, nostack, preserves_flags));
}

pub unsafe fn inl(port: u16) -> u32 {
    let ret: u32;
    asm!("in eax, dx", out("eax") ret, in("dx") port, options(nomem, nostack, preserves_flags));
    ret
}

pub unsafe fn outw(port: u16, val: u16) {
    asm!("out dx, ax", in("dx") port, in("ax") val, options(nomem, nostack, preserves_flags));
}

pub unsafe fn inw(port: u16) -> u16 {
    let ret: u16;
    asm!("in ax, dx", out("ax") ret, in("dx") port, options(nomem, nostack, preserves_flags));
    ret
}

pub fn read_config(bus: u8, slot: u8, func: u8, offset: u8) -> u32 {
    let address = 0x80000000 | ((bus as u32) << 16) | ((slot as u32) << 11) | ((func as u32) << 8) | ((offset as u32) & 0xFC);
    unsafe {
        outl(CONFIG_ADDRESS, address);
        inl(CONFIG_DATA)
    }
}

pub fn find_device(vendor_id: u16, device_id: u16) -> Option<(u8, u8, u8)> {
    for bus in 0..=255 {
        for slot in 0..32 {
            // Check if device exists
            let vendor_dev = read_config(bus, slot, 0, 0);
            if vendor_dev == 0xFFFFFFFF {
                continue; // No device
            }
            
            let v_id = (vendor_dev & 0xFFFF) as u16;
            let d_id = (vendor_dev >> 16) as u16;
            
            if v_id == vendor_id && d_id == device_id {
                return Some((bus, slot, 0));
            }
            
            // Note: properly we should check for multi-function devices and loop over func 1..8
        }
    }
    None
}

pub fn get_bar0(bus: u8, slot: u8, func: u8) -> u32 {
    read_config(bus, slot, func, 0x10) // BAR0 is at offset 0x10
}

pub fn get_irq(bus: u8, slot: u8, func: u8) -> u8 {
    let intr = read_config(bus, slot, func, 0x3C);
    (intr & 0xFF) as u8
}

pub fn discover_virtio() -> (u16, u8) {
    let (bus, slot, func) = find_device(0x1AF4, 0x1001).expect("Virtio-Blk device not found!");
    let bar0 = get_bar0(bus, slot, func);
    let io_base = (bar0 & !0x3) as u16;
    let irq = get_irq(bus, slot, func);
    (io_base, irq)
}
