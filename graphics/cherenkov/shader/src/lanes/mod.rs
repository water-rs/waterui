//! A SIMD lane executor for WGSL functions: runs one function of a naga
//! module over [`LANES`] invocations at once, the way a GPU runs a
//! subgroup.
//!
//! [`Program::compile`] flattens the function and everything it calls into
//! a structured program over whole-lane registers: every value is a run of
//! 32-bit scalar registers, each holding one value per lane, so each op is
//! a loop over [`LANES`] values the compiler vectorizes, and the
//! interpretive cost of an op is paid once per [`LANES`] invocations.
//! Control flow runs under per-lane masks — `if` and `switch` run each
//! branch for the lanes that take it, a loop iterates while any lane is
//! live, `break`, `continue`, `return` and `discard` retire lanes — so
//! divergent invocations compute exactly what they would alone.
//!
//! Lanes are laid out as 2×2 quads (lane bit 0 the column, bit 1 the row)
//! so `dpdx`, `dpdy` and `fwidth` take the quad differences a GPU takes;
//! a caller lays its invocations out to match.
//!
//! The host supplies what the module only declares: calls to the functions
//! an [`Interface`] names go to [`Host::call`] instead of their WGSL bodies,
//! and the private globals it lists are inputs the host writes, not memory
//! the program resets. Uniform globals are bound by writing their
//! registers ([`Program::global`]). Every construct the executor does not
//! model is a [`LaneError::Unsupported`] at compile time, never a wrong
//! result at run time.
//!
//! Unlike `eval` — the scalar reference interpreter, an oracle — this is an
//! executor: it allocates nothing per run.

mod compile;
mod exec;
mod ty;

use naga::Module;
use naga::valid::{Capabilities, ModuleInfo, ValidationFlags, Validator};

/// Invocations a run executes together: 8 quads.
pub const LANES: usize = 32;

/// One register: a 32-bit value per lane, stored as `f32` bits (integer
/// and boolean values are their bit patterns; `bool` is `0` or `1`).
pub type Lanes = [f32; LANES];

/// Which lanes are live.
pub type Mask = [bool; LANES];

/// A register index.
pub type Reg = u32;

/// What the host supplies beyond the module.
#[derive(Clone, Copy, Debug, Default)]
pub struct Interface<'a> {
    /// Functions whose calls go to [`Host::call`], by name, indexed in
    /// this order. Their WGSL bodies are never run.
    pub functions: &'a [&'a str],
    /// Private globals the host writes before a run, by name; the program
    /// does not reset them.
    pub inputs: &'a [&'a str],
}

/// The functions an [`Interface`] names.
pub trait Host {
    /// Calls host function `function` (its index in
    /// [`Interface::functions`]) for the lanes of `active`: `args` are the
    /// flattened arguments, `out` the flattened result to fill. Lanes
    /// outside `active` may hold any values and are ignored.
    fn call(&mut self, function: usize, args: &[Lanes], out: &mut [Lanes], active: &Mask);
}

/// Why a function could not be compiled.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum LaneError {
    /// The WGSL source failed to parse or validate.
    #[error("{0}")]
    Invalid(String),
    /// The module has no function or entry point of this name.
    #[error("no function or entry point named `{0}`")]
    Entry(String),
    /// The function uses a construct the lane executor does not model.
    #[error("the lane executor does not support {0}")]
    Unsupported(String),
}

impl LaneError {
    pub(crate) fn unsupported(what: impl Into<String>) -> Self {
        Self::Unsupported(what.into())
    }
}

/// A compiled function.
pub struct Program {
    functions: Vec<Vec<exec::Node>>,
    consts: Vec<exec::Op>,
    registers: usize,
    arguments: Vec<Box<[Reg]>>,
    result: Box<[Reg]>,
    globals: Vec<(String, Box<[Reg]>)>,
}

impl std::fmt::Debug for Program {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Program")
            .field("functions", &self.functions.len())
            .field("registers", &self.registers)
            .finish_non_exhaustive()
    }
}

/// A run's register file, reused across runs of one [`Program`].
#[derive(Debug)]
pub struct Registers(exec::State);

impl Registers {
    /// Register `reg`.
    #[must_use]
    pub fn get(&self, reg: Reg) -> &Lanes {
        &self.0.regs[reg as usize]
    }

    /// Register `reg`, to write an argument, uniform or input.
    pub fn get_mut(&mut self, reg: Reg) -> &mut Lanes {
        &mut self.0.regs[reg as usize]
    }

    /// Writes `value` into every lane of `reg`.
    pub fn splat(&mut self, reg: Reg, value: f32) {
        self.0.regs[reg as usize] = [value; LANES];
    }
}

impl Program {
    /// Compiles `entry` — an entry point or a function of `module` — and
    /// what it calls.
    ///
    /// # Errors
    /// [`LaneError::Entry`] when no such function exists,
    /// [`LaneError::Unsupported`] when it uses a construct the executor
    /// does not model.
    pub fn compile(
        module: &Module,
        info: &ModuleInfo,
        entry: &str,
        interface: &Interface<'_>,
    ) -> Result<Self, LaneError> {
        let compiled = compile::Compiler::new(module, info, interface).compile(entry)?;
        Ok(Self {
            functions: compiled.functions,
            consts: compiled.consts,
            registers: compiled.registers,
            arguments: compiled.arguments,
            result: compiled.result,
            globals: compiled.globals,
        })
    }

    /// Parses and validates `source`, then compiles `entry`.
    ///
    /// # Errors
    /// [`LaneError::Invalid`] when the source does not parse or validate;
    /// otherwise as [`Program::compile`].
    pub fn wgsl(source: &str, entry: &str, interface: &Interface<'_>) -> Result<Self, LaneError> {
        let module = naga::front::wgsl::parse_str(source)
            .map_err(|error| LaneError::Invalid(error.emit_to_string(source)))?;
        let info = Validator::new(ValidationFlags::all(), Capabilities::all())
            .validate(&module)
            .map_err(|error| LaneError::Invalid(error.emit_to_string(source)))?;
        Self::compile(&module, &info, entry, interface)
    }

    /// The function's argument count.
    #[must_use]
    pub const fn arity(&self) -> usize {
        self.arguments.len()
    }

    /// The flattened registers of argument `index`.
    ///
    /// # Panics
    /// Panics when the function has no such argument.
    #[must_use]
    pub fn argument(&self, index: usize) -> &[Reg] {
        &self.arguments[index]
    }

    /// The flattened registers of the function's result.
    #[must_use]
    pub fn result(&self) -> &[Reg] {
        &self.result
    }

    /// The flattened registers of the uniform or private global `name`.
    #[must_use]
    pub fn global(&self, name: &str) -> Option<&[Reg]> {
        self.globals
            .iter()
            .find(|(global, _)| global == name)
            .map(|(_, regs)| &**regs)
    }

    /// A register file for this program, its constants filled.
    #[must_use]
    pub fn registers(&self) -> Registers {
        let mut state = exec::State::new(self.registers);
        let code = exec::Code {
            functions: &self.functions,
        };
        let prologue: Vec<exec::Node> = self.consts.iter().cloned().map(exec::Node::Op).collect();
        code.block(&prologue, [true; LANES], &mut state, &mut NoHost);
        Registers(state)
    }

    /// Runs the function for the lanes of `active`. Returns the lanes that
    /// completed — `active` without the lanes that `discard`ed — whose
    /// result registers now hold the result.
    pub fn run(&self, registers: &mut Registers, active: Mask, host: &mut impl Host) -> Mask {
        let state = &mut registers.0;
        state.killed = [false; LANES];
        let code = exec::Code {
            functions: &self.functions,
        };
        code.block(&self.functions[0], active, state, host);
        std::array::from_fn(|l| active[l] && !state.killed[l])
    }
}

/// The prologue's host: constants call nothing.
struct NoHost;

impl Host for NoHost {
    fn call(&mut self, _: usize, _: &[Lanes], _: &mut [Lanes], _: &Mask) {
        unreachable!("the constant prologue calls no host function")
    }
}
