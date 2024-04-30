use clap::{Args as Args_, Subcommand};

use super::memory::memory_bench;
use super::nop::nop_bench;
use super::omni::omni_bench;
use super::poseidon2::poseidon2_bench;
use super::poseidon2_elf::poseidon2_elf_bench;
use super::sort::sort_bench;
use super::xor::xor_bench;

#[derive(Debug, Args_, Clone)]
#[command(args_conflicts_with_subcommands = true)]
pub struct BenchArgs {
    #[command(subcommand)]
    pub function: BenchFunction,
}

#[derive(PartialEq, Debug, Subcommand, Clone)]
pub enum BenchFunction {
    MemoryBench {
        iterations: u32,
    },
    XorBench {
        iterations: u32,
    },
    NopBench {
        iterations: u32,
    },
    Poseidon2Bench {
        input_len: u32,
    },
    /// Benchmarks (almost) every instruction.
    OmniBench {
        iterations: u32,
    },
    SortBench {
        n: u32,
    },
    Poseidon2ELFBench {
        n: u32,
    },
}

impl BenchArgs {
    pub fn run(&self) -> Result<(), anyhow::Error> {
        match self.function {
            BenchFunction::MemoryBench { iterations } => memory_bench(iterations),
            BenchFunction::NopBench { iterations } => nop_bench(iterations),
            BenchFunction::Poseidon2Bench { input_len } => poseidon2_bench(input_len),
            BenchFunction::XorBench { iterations } => xor_bench(iterations),
            BenchFunction::OmniBench { iterations } => omni_bench(iterations),
            BenchFunction::SortBench { n } => sort_bench(n),
            BenchFunction::Poseidon2ELFBench { n } => poseidon2_elf_bench(n),
        }
    }
}
