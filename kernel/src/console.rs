use crate::capability::{CPtr, CapRights, CapType};
#[cfg(not(test))]
use core::arch::asm;

#[cfg(not(test))]
const COM1_DATA: u16 = 0x3F8;
#[cfg(not(test))]
const COM1_LSR: u16 = 0x3FD;

#[cfg(not(test))]
const PS2_DATA: u16 = 0x60;
#[cfg(not(test))]
const PS2_STATUS: u16 = 0x64;

const BUFFER_CAPACITY: usize = 512;

static mut INPUT_BUFFER: [u8; BUFFER_CAPACITY] = [0; BUFFER_CAPACITY];
static mut HEAD: usize = 0;
static mut TAIL: usize = 0;
#[cfg(not(test))]
static mut SHIFT_ACTIVE: bool = false;

#[cfg(not(test))]
#[inline(always)]
unsafe fn inb(port: u16) -> u8 {
    let mut val: u8;
    asm!("in al, dx", out("al") val, in("dx") port, options(nomem, nostack, preserves_flags));
    val
}

#[cfg(not(test))]
#[inline(always)]
unsafe fn outb(port: u16, val: u8) {
    asm!("out dx, al", in("dx") port, in("al") val, options(nomem, nostack, preserves_flags));
}

/// Enqueues a character into the circular input buffer.
pub fn enqueue_char(c: u8) {
    unsafe {
        let next = (HEAD + 1) % BUFFER_CAPACITY;
        if next != TAIL {
            INPUT_BUFFER[HEAD] = c;
            HEAD = next;
        }
    }
}

/// Dequeues a character from the circular input buffer.
pub fn dequeue_char() -> Option<u8> {
    unsafe {
        if HEAD == TAIL {
            None
        } else {
            let c = INPUT_BUFFER[TAIL];
            TAIL = (TAIL + 1) % BUFFER_CAPACITY;
            Some(c)
        }
    }
}

/// Translates a PS/2 scancode (Set 1) to an ASCII byte.
fn ps2_scancode_to_ascii(scancode: u8, shift: bool) -> Option<u8> {
    match scancode {
        0x01 => Some(27), // Esc
        0x02 => Some(if shift { b'!' } else { b'1' }),
        0x03 => Some(if shift { b'@' } else { b'2' }),
        0x04 => Some(if shift { b'#' } else { b'3' }),
        0x05 => Some(if shift { b'$' } else { b'4' }),
        0x06 => Some(if shift { b'%' } else { b'5' }),
        0x07 => Some(if shift { b'^' } else { b'6' }),
        0x08 => Some(if shift { b'&' } else { b'7' }),
        0x09 => Some(if shift { b'*' } else { b'8' }),
        0x0A => Some(if shift { b'(' } else { b'9' }),
        0x0B => Some(if shift { b')' } else { b'0' }),
        0x0C => Some(if shift { b'_' } else { b'-' }),
        0x0D => Some(if shift { b'+' } else { b'=' }),
        0x0E => Some(8), // Backspace
        0x0F => Some(b'\t'),
        0x10 => Some(if shift { b'Q' } else { b'q' }),
        0x11 => Some(if shift { b'W' } else { b'w' }),
        0x12 => Some(if shift { b'E' } else { b'e' }),
        0x13 => Some(if shift { b'R' } else { b'r' }),
        0x14 => Some(if shift { b'T' } else { b't' }),
        0x15 => Some(if shift { b'Y' } else { b'y' }),
        0x16 => Some(if shift { b'U' } else { b'u' }),
        0x17 => Some(if shift { b'I' } else { b'i' }),
        0x18 => Some(if shift { b'O' } else { b'o' }),
        0x19 => Some(if shift { b'P' } else { b'p' }),
        0x1A => Some(if shift { b'{' } else { b'[' }),
        0x1B => Some(if shift { b'}' } else { b']' }),
        0x1C => Some(b'\n'), // Enter
        0x1E => Some(if shift { b'A' } else { b'a' }),
        0x1F => Some(if shift { b'S' } else { b's' }),
        0x20 => Some(if shift { b'D' } else { b'd' }),
        0x21 => Some(if shift { b'F' } else { b'f' }),
        0x22 => Some(if shift { b'G' } else { b'g' }),
        0x23 => Some(if shift { b'H' } else { b'h' }),
        0x24 => Some(if shift { b'J' } else { b'j' }),
        0x25 => Some(if shift { b'K' } else { b'k' }),
        0x26 => Some(if shift { b'L' } else { b'l' }),
        0x27 => Some(if shift { b':' } else { b';' }),
        0x28 => Some(if shift { b'"' } else { b'\'' }),
        0x29 => Some(if shift { b'~' } else { b'`' }),
        0x2B => Some(if shift { b'|' } else { b'\\' }),
        0x2C => Some(if shift { b'Z' } else { b'z' }),
        0x2D => Some(if shift { b'X' } else { b'x' }),
        0x2E => Some(if shift { b'C' } else { b'c' }),
        0x2F => Some(if shift { b'V' } else { b'v' }),
        0x30 => Some(if shift { b'B' } else { b'b' }),
        0x31 => Some(if shift { b'N' } else { b'n' }),
        0x32 => Some(if shift { b'M' } else { b'm' }),
        0x33 => Some(if shift { b'<' } else { b',' }),
        0x34 => Some(if shift { b'>' } else { b'.' }),
        0x35 => Some(if shift { b'?' } else { b'/' }),
        0x39 => Some(b' '), // Space
        _ => None,
    }
}

#[cfg(not(test))]
static mut LAST_WAS_CR: bool = false;

#[cfg(test)]
pub static mut TEST_CONSOLE_OUT_COUNT: usize = 0;

/// Polls COM1 serial and PS/2 keyboard ports for pending input characters.
pub fn poll_input() {
    #[cfg(not(test))]
    unsafe {
        // 1. Poll Serial COM1
        // LSR bit 0 (0x01) indicates data ready in RBR
        while (inb(COM1_LSR) & 0x01) != 0 {
            let byte = inb(COM1_DATA);
            if byte == b'\r' {
                LAST_WAS_CR = true;
                enqueue_char(b'\n');
            } else if byte == b'\n' {
                if LAST_WAS_CR {
                    LAST_WAS_CR = false;
                } else {
                    enqueue_char(b'\n');
                }
            } else {
                LAST_WAS_CR = false;
                enqueue_char(byte);
            }
        }

        // 2. Poll PS/2 Keyboard
        // Status port 0x64 bit 0 (0x01) indicates output buffer full
        while {
            let status = inb(PS2_STATUS);
            if (status & 0x01) != 0 {
                let scancode = inb(PS2_DATA);
                // Bit 5 (0x20) indicates aux/mouse data; only process if from keyboard
                if (status & 0x20) == 0 {
                    if scancode == 0x2A || scancode == 0x36 {
                        SHIFT_ACTIVE = true;
                    } else if scancode == 0xAA || scancode == 0xB6 {
                        SHIFT_ACTIVE = false;
                    } else if scancode < 0x80 {
                        // Key press
                        if let Some(ch) = ps2_scancode_to_ascii(scancode, SHIFT_ACTIVE) {
                            enqueue_char(ch);
                        }
                    }
                }
                true
            } else {
                false
            }
        } {}
    }
}

fn check_console_cap(cptr: CPtr, required_rights: CapRights) -> Result<(), u64> {
    let task_id = match crate::task::current_task_id() {
        Some(id) => id as u8,
        None => return Err(!0),
    };
    let slot = match unsafe { crate::capability::lookup_slot(task_id, cptr) } {
        Ok(s) => s,
        Err(_) => return Err(!0),
    };
    if slot.cap_type != CapType::Console || !slot.rights.contains(required_rights) {
        return Err(!0);
    }
    Ok(())
}

/// Syscall 24: Reads up to `max_len` bytes from the console input buffer into user space.
///
/// Returns the number of bytes read. If no bytes are available, returns 0 immediately.
pub fn sys_read_console(cptr: CPtr, buf_ptr: u64, max_len: u64) -> u64 {
    if check_console_cap(cptr, CapRights::READ).is_err() {
        return !0;
    }
    if max_len == 0 {
        return 0;
    }
    poll_input();
    let mut temp = [0u8; 128];
    let limit = core::cmp::min(max_len as usize, temp.len());
    let mut count = 0;

    while count < limit {
        if let Some(c) = dequeue_char() {
            temp[count] = c;
            count += 1;
        } else {
            break;
        }
    }

    if count > 0 && crate::ipc::copy_user_slice_to(buf_ptr, &temp[..count]).is_err() {
        return !0;
    }
    count as u64
}

/// Syscall 25: Writes `len` bytes from user space buffer to the console (COM1).
///
/// Returns the number of bytes written.
pub fn sys_write_console(cptr: CPtr, buf_ptr: u64, len: u64) -> u64 {
    if check_console_cap(cptr, CapRights::WRITE).is_err() {
        return !0;
    }
    if len == 0 {
        return 0;
    }
    let mut temp = [0u8; 256];
    let mut remaining = len as usize;
    let mut offset = 0;

    while remaining > 0 {
        let chunk = core::cmp::min(remaining, temp.len());
        let current_ptr = match buf_ptr.checked_add(offset as u64) {
            Some(p) => p,
            None => return !0,
        };
        if crate::ipc::copy_user_slice_from(current_ptr, &mut temp[..chunk]).is_err() {
            return !0;
        }
        for &b in &temp[..chunk] {
            #[cfg(not(test))]
            unsafe {
                // Wait for transmitter holding register empty (LSR bit 5 = 0x20)
                while (inb(COM1_LSR) & 0x20) == 0 {}
                outb(COM1_DATA, b);
            }
            #[cfg(test)]
            unsafe {
                let _ = b;
                TEST_CONSOLE_OUT_COUNT += 1;
            }
        }
        offset += chunk;
        remaining -= chunk;
    }

    len
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_console_queue_and_dequeue() {
        enqueue_char(b'A');
        enqueue_char(b'B');
        enqueue_char(b'C');
        assert_eq!(dequeue_char(), Some(b'A'));
        assert_eq!(dequeue_char(), Some(b'B'));
        assert_eq!(dequeue_char(), Some(b'C'));
        assert_eq!(dequeue_char(), None);
    }

    #[test]
    fn test_ps2_scancode_translation() {
        assert_eq!(ps2_scancode_to_ascii(0x1E, false), Some(b'a'));
        assert_eq!(ps2_scancode_to_ascii(0x1E, true), Some(b'A'));
        assert_eq!(ps2_scancode_to_ascii(0x1C, false), Some(b'\n'));
        assert_eq!(ps2_scancode_to_ascii(0x39, false), Some(b' '));
    }
}
