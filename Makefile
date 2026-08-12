.PHONY: all bootloader kernel run clean

QEMU = qemu-system-x86_64
OVMF = /usr/share/ovmf/OVMF.fd

all: bootloader kernel

bootloader:
	cd bootloader && cargo build --target x86_64-unknown-uefi -Zbuild-std=core,compiler_builtins,alloc

kernel:
	cd kernel && cargo build --target x86_64-unknown-none -Zbuild-std=core,compiler_builtins,alloc

run: all
	mkdir -p esp/EFI/BOOT
	cp target/x86_64-unknown-uefi/debug/bootloader.efi esp/EFI/BOOT/BOOTX64.EFI
	cp target/x86_64-unknown-none/debug/kernel esp/kernel.elf
	dd if=/dev/zero of=test.img bs=1M count=1
	printf '\xBE\xBA\xFE\xCA' | dd of=test.img conv=notrunc
	$(QEMU) -nographic -no-reboot -drive if=pflash,format=raw,readonly=on,file=$(OVMF) \
		-drive format=raw,file=fat:rw:esp \
		-drive file=test.img,format=raw,if=virtio

clean:
	cargo clean
	rm -rf esp
