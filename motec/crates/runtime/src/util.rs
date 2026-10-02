pub(crate) fn decode_r3(operands: u32) -> (u8, u8, u8) {
    let a: u8 = (operands & 0xFF) as u8;
    let b: u8 = ((operands >> 8) & 0xFF) as u8;
    let c: u8 = ((operands >> 16) & 0xFF) as u8;
    (a, b, c)
}

pub(crate) fn decode_r2(operands: u32) -> (u8, u8) {
    let a: u8 = (operands & 0xFF) as u8;
    let b: u8 = ((operands >> 8) & 0xFF) as u8;
    (a, b)
}

pub(crate) fn decode_ri(operands: u32) -> (u8, u16) {
    let a: u8 = (operands & 0xFF) as u8;
    let b: u16 = ((operands >> 8) & 0xFFFF) as u16;
    (a, b)
}

pub(crate) fn decode_ju(operands: u32) -> i32{
    ((operands << 8) as i32) >> 8
}

pub(crate) fn decode_jc(operands: u32) -> (u8, i32) {
    let a = (operands & 0xFF) as u8;
    let sbx = (((operands >> 8) as u16) as i16) as i32;
    (a, sbx)
}

