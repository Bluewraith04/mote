//! The instruction set: opcodes, their encoding, the 16-byte `Value` and type descriptors.
/// `Value`, object headers, type descriptors and the reserved type ids.
pub mod value;
pub mod equality;
/// The opcodes.
pub mod opcode;
/// Instruction encoding and decoding.
pub mod encoding;
pub mod seal;
pub mod intrinsics;
pub mod code;
pub mod type_term;
