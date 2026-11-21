//! Lowers the rows of channels: the ends are made and closed by the runtime, and a message is copied between a stack slot and the channel's buffer there.

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{types, InstBuilder, Value};
use cranelift_module::FuncId as ClifFunc;
use mote_ir::{Inst, Operand};
use mote_semantics::{TyId, TyKind};

use crate::function::Lowering;
use crate::scalar::Num;
use crate::values::internal;
use crate::CompileError;

/// The `Int` a channel buffer is given as.
const INT: Num = Num::Int { bits: 64, signed: true };

impl Lowering<'_, '_, '_> {
    /// The message type of a `Sender` or `Receiver` operand.
    fn message_of(&self, end: TyId) -> Result<TyId, CompileError> {
        let message = match self.env.program.types.kind(end) {
            TyKind::Adt { args, .. } => args.first().copied().ok_or_else(|| internal("a channel end without a message type"))?,
            _ => return Err(internal("a channel call on a value that is not a channel end")),
        };
        if matches!(self.env.program.types.kind(message), TyKind::Optional(_)) {
            return Err(internal("a channel of optional messages, which the checker refuses"));
        }
        Ok(message)
    }

    /// The rows of channels.
    pub(crate) fn channel_intrinsic(&mut self, name: &str, inst: &Inst, args: &[Operand]) -> Result<(), CompileError> {
        match (name, args) {
            ("channel_new", [buffer]) => self.channel_new(inst, buffer),
            ("port_open", [mode]) => self.port_open(inst, mode),
            ("sender_send", [tx, value]) => {
                self.send(tx, value, true)?;
                Ok(())
            }
            ("sender_try_send", [tx, value]) => {
                let sent = self.send(tx, value, false)?;
                let sent = self.b.ins().ireduce(types::I8, sent);
                self.set(inst.dest, Some(sent));
                Ok(())
            }
            ("receiver_recv", [rx]) => self.receive(inst, rx, true, None),
            ("receiver_try_recv", [rx]) => self.receive(inst, rx, false, None),
            ("receiver_recv_timeout", [rx, ms]) => self.receive(inst, rx, true, Some(ms)),
            ("sender_close", [tx]) => self.end_call(self.env.imports.sender_close, tx).map(drop),
            ("receiver_close", [rx]) => self.end_call(self.env.imports.receiver_close, rx).map(drop),
            ("sender_clone", [tx]) => {
                let end = self.end_call(self.env.imports.sender_clone, tx)?;
                self.set(inst.dest, end);
                Ok(())
            }
            ("receiver_clone", [rx]) => {
                let end = self.end_call(self.env.imports.receiver_clone, rx)?;
                self.set(inst.dest, end);
                Ok(())
            }
            ("receiver_is_closed", [rx]) => {
                let closed = self.end_call(self.env.imports.receiver_closed, rx)?.ok_or_else(|| internal("a closed check with no result"))?;
                let closed = self.b.ins().ireduce(types::I8, closed);
                self.set(inst.dest, Some(closed));
                Ok(())
            }
            _ => Err(CompileError::internal(format!("the intrinsic `{name}` with these arguments"))),
        }
    }

    /// Calls an entry that takes one end and gives back its result, if it has one.
    fn end_call(&mut self, import: ClifFunc, end: &Operand) -> Result<Option<Value>, CompileError> {
        let end = self.num_operand(end, Num::Ref)?;
        Ok(self.entry(import, &[end]).first().copied())
    }

    /// `channel_new`: the channel, then its two ends, written in the pair.
    fn channel_new(&mut self, inst: &Inst, buffer: &Operand) -> Result<(), CompileError> {
        let dest = Self::dest_reg(inst)?;
        let TyKind::Tuple(ends) = self.env.program.types.kind(self.function.regs[dest].ty).clone() else { return Err(internal("a channel_new that does not give a pair")) };
        let [sender, receiver] = ends[..] else { return Err(internal("a channel_new that does not give two ends")) };
        let channel = self.env.types.channel(self.message_of(sender)?);
        let (sender, receiver) = (self.env.types.id(sender), self.env.types.id(receiver));
        let capacity = self.num_operand(buffer, INT)?;
        let channel_type = self.b.ins().iconst(types::I32, i64::from(channel.raw()));
        let channel = self.entry(self.env.imports.channel_new, &[channel_type, capacity])[0];
        let sender_type = self.b.ins().iconst(types::I32, i64::from(sender.raw()));
        let tx = self.entry(self.env.imports.channel_end, &[channel, sender_type])[0];
        let receiver_type = self.b.ins().iconst(types::I32, i64::from(receiver.raw()));
        let rx = self.entry(self.env.imports.channel_end, &[channel, receiver_type])[0];
        let pair = self.env.types.table.get(self.reg_type(dest));
        let (first, second) = (pair.fields[0].offset, pair.fields[1].offset);
        let at = self.reg_addr(dest)?;
        self.store_reference(tx, at.at(first));
        self.store_reference(rx, at.at(second));
        Ok(())
    }

    /// `port_open`: a channel of byte arrays, the port on it and its receiver, written in the pair; `Latest` is a queue of 0.
    fn port_open(&mut self, inst: &Inst, mode: &Operand) -> Result<(), CompileError> {
        let dest = Self::dest_reg(inst)?;
        let TyKind::Tuple(ends) = self.env.program.types.kind(self.function.regs[dest].ty).clone() else { return Err(internal("a port_open that does not give a pair")) };
        let [port, receiver] = ends[..] else { return Err(internal("a port_open that does not give two ends")) };
        let channel = self.env.types.channel(self.message_of(receiver)?);
        let (port, receiver) = (self.env.types.id(port), self.env.types.id(receiver));
        let mode = Self::reg_of(mode)?;
        let tag = self.tag_of(mode)?;
        let queue_variant = self.env.types.table.get(self.reg_type(mode)).variants.first().and_then(|variant| variant.fields.first().copied()).ok_or_else(|| internal("a port mode with no queue size"))?;
        let at = self.reg_addr(mode)?;
        let size = self.load(types::I64, at.at(queue_variant.offset));
        let one = self.b.ins().iconst(types::I64, 1);
        let size = self.b.ins().smax(size, one);
        let zero = self.b.ins().iconst(types::I64, 0);
        let is_queue = self.b.ins().icmp_imm(IntCC::Equal, tag, 0);
        let queue = self.b.ins().select(is_queue, size, zero);
        let capacity = self.b.ins().smax(queue, one);
        let channel_type = self.b.ins().iconst(types::I32, i64::from(channel.raw()));
        let channel = self.entry(self.env.imports.channel_new, &[channel_type, capacity])[0];
        let port_type = self.b.ins().iconst(types::I32, i64::from(port.raw()));
        let port = self.entry(self.env.imports.port_new, &[channel, port_type, queue])[0];
        let receiver_type = self.b.ins().iconst(types::I32, i64::from(receiver.raw()));
        let rx = self.entry(self.env.imports.channel_end, &[channel, receiver_type])[0];
        let pair = self.env.types.table.get(self.reg_type(dest));
        let (first, second) = (pair.fields[0].offset, pair.fields[1].offset);
        let at = self.reg_addr(dest)?;
        self.store_reference(port, at.at(first));
        self.store_reference(rx, at.at(second));
        Ok(())
    }

    /// Sends the value; without `wait` the result is 1 when it was sent and 0 when there was no room.
    fn send(&mut self, tx: &Operand, value: &Operand, wait: bool) -> Result<Value, CompileError> {
        let element = self.env.types.id(self.message_of(self.ty_of(tx))?);
        let again = self.b.create_block();
        let done = self.b.create_block();
        self.b.ins().jump(again, &[]);
        self.b.switch_to_block(again);
        let end = self.num_operand(tx, Num::Ref)?;
        let at = self.spill_as(value, element)?;
        let source = self.address(at);
        let wait_flag = self.b.ins().iconst(types::I64, i64::from(wait));
        let sent = self.entry(self.env.imports.channel_send, &[end, source, wait_flag])[0];
        if wait {
            // A wait that ended with the message not sent asks again.
            let retry = self.b.ins().icmp_imm(IntCC::Equal, sent, 0);
            self.b.ins().brif(retry, again, &[], done, &[]);
        } else {
            self.b.ins().jump(done, &[]);
        }
        self.b.switch_to_block(done);
        Ok(sent)
    }

    /// Receives into the optional of the message: `null` when the channel is closed and empty, or, without `wait`, when nothing is there, or when the milliseconds of `timeout` ran out.
    fn receive(&mut self, inst: &Inst, rx: &Operand, wait: bool, timeout: Option<&Operand>) -> Result<(), CompileError> {
        let dest = Self::dest_reg(inst)?;
        let element = self.env.types.id(self.message_of(self.ty_of(rx))?);
        let size = self.env.types.table.layout(element).size.max(8);
        let slot = self.temp(size);
        let again = self.b.create_block();
        let done = self.b.create_block();
        self.b.ins().jump(again, &[]);
        self.b.switch_to_block(again);
        let end = self.num_operand(rx, Num::Ref)?;
        let dest_address = self.address(slot);
        let status = match timeout {
            Some(ms) => {
                let ms = self.num_operand(ms, INT)?;
                self.entry(self.env.imports.channel_recv_timeout, &[end, dest_address, ms])[0]
            }
            None => {
                let wait_flag = self.b.ins().iconst(types::I64, i64::from(wait));
                self.entry(self.env.imports.channel_recv, &[end, dest_address, wait_flag])[0]
            }
        };
        if wait && timeout.is_none() {
            // A wait that ended with no message asks again.
            let retry = self.b.ins().icmp_imm(IntCC::Equal, status, 2);
            self.b.ins().brif(retry, again, &[], done, &[]);
        } else {
            self.b.ins().jump(done, &[]);
        }
        self.b.switch_to_block(done);
        let optional = self.env.types.table.get(self.reg_type(dest));
        if optional.is_reference() {
            let message = self.load(types::I64, slot);
            self.set(inst.dest, Some(message));
            return Ok(());
        }
        let field = optional.variants.get(1).and_then(|variant| variant.fields.first().copied()).ok_or_else(|| internal("an optional with no payload"))?;
        let at = self.reg_addr(dest)?;
        self.store(status, at);
        self.copy_value_at(slot, at.at(field.offset), field.ty)
    }
}
