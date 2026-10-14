#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// The opcodes.
pub enum Opcode {
    ADD = 0,
    SUB = 1,
    MUL = 2,
    DIV = 3,
    MOD = 4,
    NEG = 5,

    AND = 6,
    OR = 7,
    NOT = 8,
    XOR = 9,
    BAND = 10,
    BOR = 11,
    BXOR = 12,
    BNOT = 13,
    SHL = 14,
    SHR = 15,

    EQ = 16,
    NE = 17,
    LT = 18,
    LE = 19,
    GT = 20,
    GE = 21,

    NEWOBJ = 22,
    SETFIELD = 23,
    GETFIELD = 24,
    TYPEOF = 25,
    
    MOVE = 26,
    LOADI = 27,
    LOADK = 28,

    JMP = 29,
    JMPIF = 30,
    JMPIFNOT = 31,

    /// `CALL rA, bx`: calls code object `bx` over the window `rA+1..`, result in `rA`.
    CALL = 32,
    RET = 33,

    NOP = 34,
    HALT = 35,
    DEBUGPRINT = 36,

    ENTERARENA = 37,
    EXITARENA = 38,
    ARENAALLOC = 39,

    CALLNATIVE = 40,

    /// `rA = new heap string from string_table[bx]` of the current code object.
    NEWSTR = 41,

    /// Indirect call: `rA = call(value in rB)(args at rC..)`.
    CALLV = 42,

    /// `rA = the current frame's closure captures[bx]`.
    GETCAPTURE = 43,

    /// Module-level globals; `bx` indexes a flat, program-wide array.
    GETGLOBAL = 44,
    SETGLOBAL = 45,

    /// Structured concurrency: open a child-task set, wait for it, spawn a child.
    SCOPEENTER = 46,
    SCOPEEXIT = 47,
    SPAWN = 48,

    /// `COPYVAL rA, rB`: `rA = rB`, except a boxed value-type object is first deep-copied into a fresh object.
    COPYVAL = 49,

    /// Generators: `MKGEN` packs the frame into a `Generator` object.
    /// `RESUME` runs a generator until it yields or ends; `YIELD` suspends it.
    MKGEN = 50,
    RESUME = 51,
    YIELD = 52,

    /// `CALLNATIVEW rA, bx`: native `native_table[bx]` over the window `rA+1..`; the result lands in `rA`.
    CALLNATIVEW = 53,

    /// `CALLINTRINSIC rA, id, rC`: scheduler-aware intrinsic `id` over `rC..`, result in `rA`.
    CALLINTRINSIC = 54,

    /// `STAMP rA, bx`: the object in `rA` takes the descriptor `type_descriptors[bx]`.
    STAMP = 55,

    /// `LOADTYPE rA, bx`: `rA` = the interned id of the type on `type_descriptors[bx]`.
    LOADTYPE = 56,

    /// `STAMPT rA, rB`: the object in `rA` records the type whose id `rB` holds.
    STAMPT = 57,

    /// `ASTYPE rA, rB`: faults unless the value in `rA` fits the type whose id `rB` holds.
    ASTYPE = 58,

    /// `ISTYPE rA, rB, rC`: `rA` = whether the value in `rB` fits the type whose id `rC` holds.
    ISTYPE = 59,

    /// `SOME rA, rB`: `rA` = `Some(rB)`.
    SOME = 60,

    /// `UNSOME rA, rB`: `rA` = the payload of the `Some` in `rB`.
    UNSOME = 61,

    /// `RETN rA, n`: returns the `n` registers `rA..` to the caller.
    RETN = 62,

    /// `CALLNATIVEF rA, bx`: `CALLNATIVEW` for a fallible native.
    CALLNATIVEF = 63,
}

impl TryFrom<u8> for Opcode {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Opcode::ADD),
            1 => Ok(Opcode::SUB),
            2 => Ok(Opcode::MUL),
            3 => Ok(Opcode::DIV),
            4 => Ok(Opcode::MOD),
            5 => Ok(Opcode::NEG),
            6 => Ok(Opcode::AND),
            7 => Ok(Opcode::OR),
            8 => Ok(Opcode::NOT),
            9 => Ok(Opcode::XOR),
            10 => Ok(Opcode::BAND),
            11 => Ok(Opcode::BOR),
            12 => Ok(Opcode::BXOR),
            13 => Ok(Opcode::BNOT),
            14 => Ok(Opcode::SHL),
            15 => Ok(Opcode::SHR),
            16 => Ok(Opcode::EQ),
            17 => Ok(Opcode::NE),
            18 => Ok(Opcode::LT),
            19 => Ok(Opcode::LE),
            20 => Ok(Opcode::GT),
            21 => Ok(Opcode::GE),
            22 => Ok(Opcode::NEWOBJ),
            23 => Ok(Opcode::SETFIELD),
            24 => Ok(Opcode::GETFIELD),
            25 => Ok(Opcode::TYPEOF),
            26 => Ok(Opcode::MOVE),
            27 => Ok(Opcode::LOADI),
            28 => Ok(Opcode::LOADK),
            29 => Ok(Opcode::JMP),
            30 => Ok(Opcode::JMPIF),
            31 => Ok(Opcode::JMPIFNOT),
            32 => Ok(Opcode::CALL),
            33 => Ok(Opcode::RET),
            34 => Ok(Opcode::NOP),
            35 => Ok(Opcode::HALT),
            36 => Ok(Opcode::DEBUGPRINT),
            37 => Ok(Opcode::ENTERARENA),
            38 => Ok(Opcode::EXITARENA),
            39 => Ok(Opcode::ARENAALLOC),
            40 => Ok(Opcode::CALLNATIVE),
            41 => Ok(Opcode::NEWSTR),
            42 => Ok(Opcode::CALLV),
            43 => Ok(Opcode::GETCAPTURE),
            44 => Ok(Opcode::GETGLOBAL),
            45 => Ok(Opcode::SETGLOBAL),
            46 => Ok(Opcode::SCOPEENTER),
            47 => Ok(Opcode::SCOPEEXIT),
            48 => Ok(Opcode::SPAWN),
            49 => Ok(Opcode::COPYVAL),
            50 => Ok(Opcode::MKGEN),
            51 => Ok(Opcode::RESUME),
            52 => Ok(Opcode::YIELD),
            53 => Ok(Opcode::CALLNATIVEW),
            54 => Ok(Opcode::CALLINTRINSIC),
            55 => Ok(Opcode::STAMP),
            56 => Ok(Opcode::LOADTYPE),
            57 => Ok(Opcode::STAMPT),
            58 => Ok(Opcode::ASTYPE),
            59 => Ok(Opcode::ISTYPE),
            60 => Ok(Opcode::SOME),
            61 => Ok(Opcode::UNSOME),
            62 => Ok(Opcode::RETN),
            63 => Ok(Opcode::CALLNATIVEF),
            _ => Err(value),
        }
    }
}