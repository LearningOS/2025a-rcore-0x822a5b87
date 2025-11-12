# Building

# 指定目标平台，模式
TARGET := riscv64gc-unknown-none-elf
MODE := release

# 声明target的elf，bin，汇编的输出目录
KERNEL_ELF := target/$(TARGET)/$(MODE)/os
KERNEL_BIN := $(KERNEL_ELF).bin
DISASM_TMP := target/$(TARGET)/$(MODE)/asm

# Building mode argument
ifeq ($(MODE), release)
	MODE_ARG := --release
endif

# BOARD
BOARD := qemu
SBI ?= rustsbi
BOOTLOADER := ../bootloader/$(SBI)-$(BOARD).bin

# 内核的物理入口地址（PA = Physical Address），必须与链接脚本（linker.ld）中定义的代码段（.text）起始地址完全一致
# 裸机环境中，CPU 上电后从指定物理地址开始执行，QEMU 通过该地址加载内核二进制并跳转到入口
# 若地址不匹配，会导致内核执行错乱（如跳转到无效指令）
KERNEL_ENTRY_PA := 0x80200000

# Binutils
OBJDUMP := rust-objdump --arch-name=riscv64
OBJCOPY := rust-objcopy --binary-architecture=riscv64

# Disassembly
DISASM ?= -x

# 指定build的依赖
build: env $(KERNEL_BIN)

# 初始化rust编译所需的所有组件，指定target
env:
	(rustup target list | grep "riscv64gc-unknown-none-elf (installed)") || rustup target add $(TARGET)
	cargo install cargo-binutils
	rustup component add rust-src
	rustup component add llvm-tools-preview

# 调用 rust-objcopy，rust-objcopy 它将某个特定的二进制格式，转换为另外一个特定的二进制格式。
# 例如，我们在linux下通过cargo build得到了elf格式的文件，我们可以通过rust-objcopy转换为一个纯二进制的文件。
# 这样我们就从一个linux可以识别的二进制文件变成了一个裸机上可以运行的bare metal app。
#
# --strip-all : 剥离 ELF 中的所有符号表、调试信息（即使 release 模式有残留元数据也会移除）；
# -O binary：指定输出格式为「纯二进制」；
#
# $@ 是一个自动变量（automatic variable），指代当前规则中的目标
# $^ 指代当前规则的「所有依赖」
# $< 指代当前规则的「第一个依赖」
# $* 指代目标文件名的「无后缀部分」
$(KERNEL_BIN): kernel
	@$(OBJCOPY) $(KERNEL_ELF) --strip-all -O binary $@

# 编译rust内核输出到 target/riscv64gc-unknown-none-elf/release/os
# 整体由 “基础目录/目标架构目录/构建模式目录/文件名” 组成
# 基础目录    target，是rust默认的输出路径
# 目标架构目录 riscv64gc-unknown-none-elf 来自于TARGET变量
# 构建模式目录 MODE := release，cargo build --release 会把编译产物放在 release 下
kernel:
	@echo Platform: $(BOARD)
	@cargo build $(MODE_ARG)

# 清理rust编译产物
clean:
	@cargo clean

# rust-objdump 与 rust-objcopy 同属 cargo-binutils 工具集，是 Rust 对 LLVM 工具的封装（底层依赖 LLVM 工具链）
# 核心作用是「解析二进制文件（如 ELF）并以人类可读形式输出」（不修改原文件），包括：
# - 反汇编：将机器码转换为 RISC-V 汇编指令；
# - 结构分析：显示 ELF 头信息、段表、符号表等；
# 区别于 rust-objcopy（格式转换工具），rust-objdump 仅用于分析，不改变文件格式
disasm: kernel
	@$(OBJDUMP) $(DISASM) $(KERNEL_ELF) | less

# 和disasm一样的功能，只不过使用vim代替了less作为文本编辑工具
disasm-vim: kernel
	@$(OBJDUMP) $(DISASM) $(KERNEL_ELF) > $(DISASM_TMP)
	@vim $(DISASM_TMP)
	@rm $(DISASM_TMP)

# 将我们编译得到的bin文件加载到qemu执行
# qemu-system-riscv64 指定了平台为riscv64
# -machine virt
# -nographic 关闭图像化输出，并将I/O重定向到console
# -bios $(BOOTLOADER) 指定BIOS，其实这里因为我们这里使用的是riscv，所以这里应该是SBI
# -device 指定driver：
#		loader 指定了 Generic Loader
#		file 指定 driver image
#		addr 指定了程序进入的时候执行的代码基址
run: run-inner
run-inner: build
	@qemu-system-riscv64 \
		-machine virt \
		-nographic \
		-bios $(BOOTLOADER) \
		-device loader,file=$(KERNEL_BIN),addr=$(KERNEL_ENTRY_PA)

# 在 tmux 中启动调试环境：QEMU 作为 GDB 服务器运行内核，GDB 客户端加载 ELF 符号表进行调试
# QEMU 参数说明：
# -s：默认在 1234 端口启动 GDB 服务器（等价于 -gdb tcp::1234）；
# -S：启动后暂停 CPU，等待 GDB 客户端连接并发送继续执行命令（避免内核提前运行）；
# 依赖 ELF 文件的符号表（KERNEL_ELF），GDB 才能关联源码和内存地址
debug: build
	@tmux new-session -d \
		"qemu-system-riscv64 -machine virt -nographic -bios $(BOOTLOADER) -device loader,file=$(KERNEL_BIN),addr=$(KERNEL_ENTRY_PA) -s -S" && \
		tmux split-window -h "riscv64-unknown-elf-gdb -ex 'file $(KERNEL_ELF)' -ex 'set arch riscv:rv64' -ex 'target remote localhost:1234'" && \
		tmux -2 attach-session -d

# 使用qemu执行编译得到的bin文件，并且保留符号等信息用于debug
gdbserver: build
	@qemu-system-riscv64 -machine virt -nographic -bios $(BOOTLOADER) -device loader,file=$(KERNEL_BIN),addr=$(KERNEL_ENTRY_PA) -s -S

# 通过编译得到的elf文件进行debug
gdbclient:
	@riscv64-unknown-elf-gdb -ex 'file $(KERNEL_ELF)' -ex 'set arch riscv:rv64' -ex 'target remote localhost:1234'

.PHONY: build env kernel clean disasm disasm-vim run-inner gdbserver gdbclient
