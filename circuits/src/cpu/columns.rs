use itertools::Itertools;
use plonky2::field::types::Field;

use crate::bitshift::columns::Bitshift;
use crate::bitwise::columns::XorView;
use crate::columns_view::{columns_view_impl, make_col_map, NumberOfColumns};
use crate::cross_table_lookup::Column;

#[repr(C)]
#[derive(Clone, Copy, Eq, PartialEq, Debug, Default)]
pub struct OpSelectorView<T> {
    pub add: T,
    pub sub: T,
    pub xor: T,
    pub or: T,
    pub and: T,
    pub divu: T,
    pub remu: T,
    pub mul: T,
    pub mulhu: T,
    pub sll: T,
    pub slt: T,
    pub sltu: T,
    pub srl: T,
    pub jalr: T,
    pub beq: T,
    pub bne: T,
    pub ecall: T,
}

columns_view_impl!(InstructionView);
#[derive(Clone, Copy, Eq, PartialEq, Debug, Default)]
pub struct InstructionView<T> {
    /// The original instruction (+ imm_value) used for program
    /// cross-table-lookup.
    pub pc: T,

    pub ops: OpSelectorView<T>,
    pub rs1_select: [T; 32],
    pub rs2_select: [T; 32],
    pub rd_select: [T; 32],
    pub imm_value: T,
    pub branch_target: T,
}

columns_view_impl!(CpuColumnsView);
#[repr(C)]
#[derive(Clone, Copy, Eq, PartialEq, Debug, Default)]
pub struct CpuColumnsView<T> {
    pub clk: T,
    pub inst: InstructionView<T>,

    pub halt: T,

    pub op1_value: T,
    pub op2_value: T,
    pub dst_value: T,

    pub regs: [T; 32],

    pub op1_sign: T,
    pub op2_sign: T,
    pub op1_val_fixed: T,
    pub op2_val_fixed: T,
    pub cmp_abs_diff: T,
    pub cmp_diff_inv: T,
    pub less_than: T,
    pub branch_equal: T,

    pub xor: XorView<T>,

    pub bitshift: Bitshift<T>,

    pub quotient: T,
    pub remainder: T,
    pub remainder_slack: T,
    pub divisor_inv: T,
    pub divisor: T,

    // TODO: PRODUCT_LOW_BITS and PRODUCT_HIGH_BITS need range checking.
    pub multiplier: T,
    pub product_low_bits: T,
    pub product_high_bits: T,
    pub product_high_diff_inv: T,
}

make_col_map!(CpuColumnsView);

pub const NUM_CPU_COLS: usize = CpuColumnsView::<()>::NUMBER_OF_COLUMNS;

/// Column for a binary filter for our range check in the Mozak
/// [`CpuTable`](crate::cross_table_lookup::CpuTable).
#[must_use]
pub fn filter_for_rangecheck<F: Field>() -> Column<F> { Column::single(MAP.inst.ops.add) }

/// Columns containing the data to be range checked in the Mozak
/// [`CpuTable`](crate::cross_table_lookup::CpuTable).
#[must_use]
pub fn data_for_rangecheck<F: Field>() -> Vec<Column<F>> { vec![Column::single(MAP.dst_value)] }

/// Columns containing the data to be matched against XOR Bitwise stark.
/// [`CpuTable`](crate::cross_table_lookup::CpuTable).
#[must_use]
pub fn data_for_bitwise<F: Field>() -> Vec<Column<F>> { Column::singles(MAP.xor).collect_vec() }

/// Column for a binary filter for bitwise instruction in Bitwise stark.
/// [`CpuTable`](crate::cross_table_lookup::CpuTable).
#[must_use]
pub fn filter_for_bitwise<F: Field>() -> Column<F> { Column::many(MAP.inst.ops.ops_that_use_xor()) }

impl<T: Copy> OpSelectorView<T> {
    #[must_use]
    pub fn ops_that_use_xor(&self) -> [T; 5] {
        // TODO: Add SRA, once we implement its constraints.
        [self.xor, self.or, self.and, self.srl, self.sll]
    }

    // TODO: Add SRA, once we implement its constraints.
    pub fn ops_that_shift(&self) -> [T; 2] { [self.sll, self.srl] }
}

/// Columns containing the data to be matched against `ShiftAmount` stark.
/// [`CpuTable`](crate::cross_table_lookup::CpuTable).
#[must_use]
pub fn data_for_shift_amount<F: Field>() -> Vec<Column<F>> {
    Column::singles(MAP.bitshift).collect_vec()
}

/// Column for a binary filter for shft instruction in `ShiftAmount` stark.
/// [`CpuTable`](crate::cross_table_lookup::CpuTable).
#[must_use]
pub fn filter_for_shift_amount<F: Field>() -> Column<F> {
    Column::many(MAP.inst.ops.ops_that_shift())
}
