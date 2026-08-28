//! Chip-architecture-aware properties for the ESP32 (Dew) backend.
//!
//! The ESP32 family spans two instruction set architectures: the original
//! Xtensa cores (`esp32`, `esp32s2`, `esp32s3`) and the newer RISC-V cores
//! (`esp32c3`, `esp32c6`, ...). Almost everything the CLI does for an ESP32
//! target — the Rust target triple, the QEMU system binary and machine model,
//! whether QEMU needs the eFuse ADC-calibration workaround, the firmware
//! console transport, the flash size, the main-task stack, and the codegen
//! optimization level — follows directly from the chip's architecture.
//!
//! [`Esp32Chip`] parses a chip string once and answers all of those questions,
//! so the rest of the backend never has to special-case a chip by name.

use std::str::FromStr;

use color_eyre::eyre::{Result, eyre};
use target_lexicon::{Architecture, Riscv32Architecture, Triple};

/// The instruction set architecture of an ESP32-class chip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Esp32Arch {
    /// Tensilica Xtensa LX (`esp32`, `esp32s2`, `esp32s3`).
    Xtensa,
    /// RISC-V (`esp32c3`, `esp32c6`, `esp32h2`, ...).
    RiscV,
}

/// A supported ESP32-class target chip.
///
/// The variant determines the chip's architecture and every architecture- and
/// chip-specific build/emulation parameter. Parse one with
/// [`Esp32Chip::from_str`]; the chip string is the single source of truth
/// (`[backends.esp32] chip` in `Water.toml`, or the selected platform).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Esp32Chip {
    /// ESP32-S3: dual-core Xtensa LX7.
    Esp32S3,
    /// ESP32-C3: single-core RISC-V (RV32IMC).
    Esp32C3,
    /// ESP32-P4: dual-core RISC-V (RV32IMAFC, hardware single-precision FPU).
    Esp32P4,
}

impl Esp32Chip {
    /// The canonical chip identifier (e.g. `"esp32s3"`), as used by `espflash`,
    /// QEMU machine models, and `Water.toml`.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Esp32S3 => "esp32s3",
            Self::Esp32C3 => "esp32c3",
            Self::Esp32P4 => "esp32p4",
        }
    }

    /// The chip's instruction set architecture.
    #[must_use]
    pub const fn arch(self) -> Esp32Arch {
        match self {
            Self::Esp32S3 => Esp32Arch::Xtensa,
            Self::Esp32C3 | Self::Esp32P4 => Esp32Arch::RiscV,
        }
    }

    /// The Rust target triple to cross-compile the firmware for.
    ///
    /// Xtensa chips use a per-chip triple (`xtensa-<chip>-espidf`); RISC-V
    /// chips use the architecture-level triple matching their ISA extensions
    /// (`imc` on the C-series, `imafc` — hardware single-float — on the P4).
    #[must_use]
    pub const fn target_triple(self) -> &'static str {
        match self {
            Self::Esp32S3 => "xtensa-esp32s3-espidf",
            Self::Esp32C3 => "riscv32imc-esp-espidf",
            Self::Esp32P4 => "riscv32imafc-esp-espidf",
        }
    }

    /// The target triple parsed into a [`Triple`].
    ///
    /// # Panics
    ///
    /// Panics only if the static triple stops being a valid `target_lexicon`
    /// triple, which would be a build-time bug in this table.
    #[must_use]
    pub fn triple(self) -> Triple {
        Triple::from_str(self.target_triple())
            .unwrap_or_else(|error| panic!("{} target triple must be valid: {error}", self.id()))
    }

    /// The `target_lexicon` architecture for the chip.
    #[must_use]
    pub const fn lexicon_arch(self) -> Architecture {
        match self {
            Self::Esp32S3 => Architecture::XTensa,
            Self::Esp32C3 => Architecture::Riscv32(Riscv32Architecture::Riscv32imc),
            Self::Esp32P4 => Architecture::Riscv32(Riscv32Architecture::Riscv32imafc),
        }
    }

    /// The QEMU system binary that emulates this chip's architecture.
    #[must_use]
    pub const fn qemu_binary(self) -> &'static str {
        match self.arch() {
            Esp32Arch::Xtensa => "qemu-system-xtensa",
            Esp32Arch::RiscV => "qemu-system-riscv32",
        }
    }

    /// The QEMU `-machine` model for this chip (the chip id doubles as the
    /// machine name in Espressif's QEMU fork).
    #[must_use]
    pub const fn qemu_machine(self) -> &'static str {
        self.id()
    }

    /// Whether QEMU needs the eFuse ADC-calibration workaround.
    ///
    /// Xtensa chips hang at startup in hardware ADC self-calibration, which
    /// QEMU does not emulate; an eFuse image with calibration version 1 makes
    /// startup read the (zeroed) calibration codes instead. RISC-V chips boot
    /// cleanly under QEMU without it.
    #[must_use]
    pub const fn needs_qemu_efuse_workaround(self) -> bool {
        matches!(self.arch(), Esp32Arch::Xtensa)
    }

    /// The espup toolchain component directory holding the chip's GCC, relative
    /// to `~/.rustup/toolchains/esp/<component>`, together with the `bin`
    /// subpath under the discovered version directory.
    #[must_use]
    pub const fn gcc_component(self) -> Esp32GccComponent {
        match self.arch() {
            Esp32Arch::Xtensa => Esp32GccComponent {
                component: "xtensa-esp-elf",
                bin_subpath: "xtensa-esp-elf/bin",
                what: "Xtensa GCC toolchain",
            },
            Esp32Arch::RiscV => Esp32GccComponent {
                component: "riscv32-esp-elf",
                bin_subpath: "riscv32-esp-elf/bin",
                what: "RISC-V GCC toolchain",
            },
        }
    }

    /// The harness firmware parameters that vary by chip: console transport,
    /// flash size, main-task stack, app-partition size, and codegen profile.
    #[must_use]
    pub const fn firmware_params(self) -> Esp32FirmwareParams {
        match self {
            // Real S3 devkits expose the USB-Serial-JTAG console; 8 MB flash,
            // generous stack for the Xtensa rasterization stack, and size-
            // optimized codegen to dodge the Xtensa LLVM miscompile.
            Self::Esp32S3 => Esp32FirmwareParams {
                console_uart_default: false,
                flash_size_mb: 8,
                main_task_stack_bytes: 163_840,
                app_partition_offset: "0x10000",
                app_partition_size: "0x600000",
                opt_level: "s",
            },
            // The C3 is RISC-V (mainline LLVM backend, no miscompile), so it
            // builds at -O2. QEMU surfaces UART0, and the chip has ~400 KB
            // SRAM, so the main task gets a ~48 KB stack against a small panel.
            Self::Esp32C3 => Esp32FirmwareParams {
                console_uart_default: true,
                flash_size_mb: 4,
                main_task_stack_bytes: 49_152,
                app_partition_offset: "0x10000",
                app_partition_size: "0x300000",
                opt_level: "2",
            },
            // The P4 is the RISC-V flagship: dual 400 MHz cores with a
            // hardware single-precision FPU, 768 KB of L2 memory, and
            // MIPI-DSI/parallel-RGB LCD peripherals. Mainline LLVM, so full
            // optimization; boards expose USB-Serial-JTAG and ship 16 MB
            // flash, and the roomy SRAM affords a 96 KB main task stack.
            Self::Esp32P4 => Esp32FirmwareParams {
                console_uart_default: false,
                flash_size_mb: 16,
                main_task_stack_bytes: 98_304,
                app_partition_offset: "0x10000",
                app_partition_size: "0xC00000",
                opt_level: "2",
            },
        }
    }
}

impl FromStr for Esp32Chip {
    type Err = color_eyre::eyre::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "esp32s3" => Ok(Self::Esp32S3),
            "esp32c3" => Ok(Self::Esp32C3),
            "esp32p4" => Ok(Self::Esp32P4),
            other => Err(eyre!(
                "unsupported ESP32 chip {other:?}. Supported chips: esp32s3, esp32c3, esp32p4."
            )),
        }
    }
}

/// Location of a chip's GCC toolchain within the espup `esp` toolchain.
#[derive(Debug, Clone, Copy)]
pub struct Esp32GccComponent {
    /// Component directory under `~/.rustup/toolchains/esp/`.
    pub component: &'static str,
    /// `bin` directory relative to the discovered version directory.
    pub bin_subpath: &'static str,
    /// Human-readable name for diagnostics.
    pub what: &'static str,
}

/// Chip-specific firmware harness parameters.
///
/// These are baked into the generated `sdkconfig.defaults`, `partitions.csv`,
/// and `Cargo.toml` codegen profile, and passed to `espflash` at image-merge
/// time, so the same harness template renders correctly for any chip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Esp32FirmwareParams {
    /// Route the firmware console to UART0 (`true`) or USB-Serial-JTAG
    /// (`false`). QEMU surfaces UART0, so emulated chips must use it.
    pub console_uart_default: bool,
    /// Flash size in megabytes (drives `CONFIG_ESPTOOLPY_FLASHSIZE_*MB` and the
    /// `espflash --flash-size` argument).
    pub flash_size_mb: u32,
    /// Main-task stack size in bytes (`CONFIG_ESP_MAIN_TASK_STACK_SIZE`).
    pub main_task_stack_bytes: u32,
    /// Offset of the app (`factory`) partition.
    pub app_partition_offset: &'static str,
    /// Size of the app (`factory`) partition.
    pub app_partition_size: &'static str,
    /// Cargo codegen `opt-level` for the firmware profiles.
    pub opt_level: &'static str,
}

impl Esp32FirmwareParams {
    /// The `espflash --flash-size` value (e.g. `"8mb"`).
    #[must_use]
    pub fn flash_size_arg(&self) -> String {
        format!("{}mb", self.flash_size_mb)
    }
}

#[cfg(test)]
mod tests {
    use super::{Esp32Arch, Esp32Chip};
    use std::str::FromStr;

    #[test]
    fn parses_known_chips_and_rejects_unknown() {
        assert_eq!(
            Esp32Chip::from_str("esp32s3").expect("esp32s3 parses"),
            Esp32Chip::Esp32S3
        );
        assert_eq!(
            Esp32Chip::from_str("esp32c3").expect("esp32c3 parses"),
            Esp32Chip::Esp32C3
        );
        assert_eq!(
            Esp32Chip::from_str("esp32p4").expect("esp32p4 parses"),
            Esp32Chip::Esp32P4
        );
        assert!(Esp32Chip::from_str("esp32c6").is_err());
    }

    #[test]
    fn esp32p4_derives_riscv_fpu_build_parameters() {
        let p4 = Esp32Chip::Esp32P4;
        assert_eq!(p4.arch(), Esp32Arch::RiscV);
        assert_eq!(p4.target_triple(), "riscv32imafc-esp-espidf");
        assert_eq!(p4.qemu_binary(), "qemu-system-riscv32");
        assert_eq!(p4.qemu_machine(), "esp32p4");
        assert!(!p4.needs_qemu_efuse_workaround());
        assert_eq!(p4.gcc_component().component, "riscv32-esp-elf");
        let params = p4.firmware_params();
        assert!(!params.console_uart_default);
        assert_eq!(params.flash_size_mb, 16);
        assert_eq!(params.opt_level, "2");
    }

    #[test]
    fn xtensa_and_riscv_derive_distinct_build_parameters() {
        let s3 = Esp32Chip::Esp32S3;
        assert_eq!(s3.arch(), Esp32Arch::Xtensa);
        assert_eq!(s3.target_triple(), "xtensa-esp32s3-espidf");
        assert_eq!(s3.qemu_binary(), "qemu-system-xtensa");
        assert_eq!(s3.qemu_machine(), "esp32s3");
        assert!(s3.needs_qemu_efuse_workaround());
        assert_eq!(s3.gcc_component().component, "xtensa-esp-elf");
        assert!(!s3.firmware_params().console_uart_default);
        assert_eq!(s3.firmware_params().flash_size_arg(), "8mb");

        let c3 = Esp32Chip::Esp32C3;
        assert_eq!(c3.arch(), Esp32Arch::RiscV);
        assert_eq!(c3.target_triple(), "riscv32imc-esp-espidf");
        assert_eq!(c3.qemu_binary(), "qemu-system-riscv32");
        assert_eq!(c3.qemu_machine(), "esp32c3");
        assert!(!c3.needs_qemu_efuse_workaround());
        assert_eq!(c3.gcc_component().component, "riscv32-esp-elf");
        assert!(c3.firmware_params().console_uart_default);
        assert_eq!(c3.firmware_params().flash_size_arg(), "4mb");
    }

    #[test]
    fn triple_parses_for_every_chip() {
        for chip in [Esp32Chip::Esp32S3, Esp32Chip::Esp32C3] {
            let triple = chip.triple();
            assert_eq!(triple.architecture, chip.lexicon_arch());
        }
    }
}
