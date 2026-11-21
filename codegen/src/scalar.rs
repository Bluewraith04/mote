//! The numbers and `Bool` that fit one register.

use cranelift_codegen::ir::{types, Type};
use mote_abi::Scalar;

/// A value that is one Cranelift scalar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Num {
    Int { bits: u32, signed: bool },
    Bool,
    F32,
    F64,
    /// A reference to an object, or null.
    Ref,
}

impl Num {
    pub fn clif(self) -> Type {
        match self {
            Num::Bool | Num::Int { bits: 8, .. } => types::I8,
            Num::Int { bits: 16, .. } => types::I16,
            Num::Int { bits: 32, .. } => types::I32,
            Num::Int { .. } | Num::Ref => types::I64,
            Num::F32 => types::F32,
            Num::F64 => types::F64,
        }
    }

    /// The register class a scalar of the table holds.
    pub fn from_scalar(scalar: Scalar) -> Num {
        match scalar {
            Scalar::I8 => Num::Int { bits: 8, signed: true },
            Scalar::I16 => Num::Int { bits: 16, signed: true },
            Scalar::I32 => Num::Int { bits: 32, signed: true },
            Scalar::I64 => Num::Int { bits: 64, signed: true },
            Scalar::F32 => Num::F32,
            Scalar::F64 => Num::F64,
            Scalar::Ref => Num::Ref,
        }
    }
}
