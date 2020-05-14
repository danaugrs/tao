use std::fmt;
use super::Value;

pub type CodeAddr = u32;
pub type ConstAddr = u32;

#[derive(Copy, Clone)]
#[repr(u32)]
pub enum Instr {
    Nop = 0,

    // Duplicate the last item on the stack
    Dup,
    // Pop the last item on the stack
    Pop,

    /// Push the given integer, converted to a `Value::Number`
    Integer(i32),
    /// Push the given float, converted to a `Value::Number`
    // TODO: Not precise enough
    Float(f32),
    /// Push `true`
    True,
    /// Push `false`
    False,

    /// Make a list from the last N items on the stack (reversed)
    MakeList(u32),
    // Index the list at the top of the stack
    IndexList(u32),

    NegNum,
    NotBool,
    AddNum,
    SubNum,
    MulNum,
    DivNum,
    RemNum,

    EqNum,

    /// Load a constant from the program constants
    LoadConst(ConstAddr),
    /// Push a copy of the local with the given offset on to the stack
    LoadLocal(u32),
    /// Push the top value in the stack on to the local stack
    PushLocal,
    /// Pop the last local from the local stack
    PopLocal,

    /// Jump to the address
    Jump(u32),
    /// Jump to the address if the last value in the stack is `false`
    JumpIfNot(u32),
    /// Pop the top value in the stack, consider this a return value
    /// Then, pop N additional items and return to the last pushed
    /// address, and push the return value
    Return(u32),
}

impl fmt::Debug for Instr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Instr::Nop => write!(f, "nop"),
            Instr::Dup => write!(f, "dup"),
            Instr::Pop => write!(f, "pop"),
            Instr::Integer(x) => write!(f, "int {}", x),
            Instr::Float(x) => write!(f, "float {}", x),
            Instr::True => write!(f, "true"),
            Instr::False => write!(f, "false"),
            Instr::MakeList(n) => write!(f, "list.make {}", n),
            Instr::IndexList(x) => write!(f, "list.index {}", x),
            Instr::NegNum => write!(f, "num.neg"),
            Instr::NotBool => write!(f, "bool.not"),
            Instr::AddNum => write!(f, "num.add"),
            Instr::SubNum => write!(f, "num.sub"),
            Instr::MulNum => write!(f, "num.mul"),
            Instr::DivNum => write!(f, "num.div"),
            Instr::RemNum => write!(f, "num.rem"),
            Instr::EqNum => write!(f, "num.eq"),
            Instr::LoadConst(addr) => write!(f, "const {:#X}", addr),
            Instr::LoadLocal(offset) => write!(f, "load_local {}", offset),
            Instr::PushLocal => write!(f, "push_local"),
            Instr::PopLocal => write!(f, "pop_local"),
            Instr::Jump(addr) => write!(f, "jump {:#X}", addr),
            Instr::JumpIfNot(addr) => write!(f, "jump_if_not {:#X}", addr),
            Instr::Return(n) => write!(f, "return {}", n),
        }
    }
}

#[test]
fn size() {
    assert!(std::mem::size_of::<Instr>() <= 8);
}

#[derive(Default)]
pub struct Program {
    code: Vec<Instr>,
    consts: Vec<Value>,
    entry: CodeAddr,
}

impl Program {
    pub fn entry(&self) -> CodeAddr {
        self.entry
    }

    pub unsafe fn fetch_instr_unchecked(&self, addr: CodeAddr) -> Instr {
        debug_assert!((addr as usize) < self.code.len(), "addr = {}, len = {}", addr, self.code.len());
        *self.code.get_unchecked(addr as usize)
    }

    pub fn fetch_const(&self, addr: ConstAddr) -> Value {
        self.consts[addr as usize].clone()
    }

    pub fn emit_const(&mut self, c: Value) -> ConstAddr {
        self.consts.push(c);
        (self.consts.len() - 1) as ConstAddr
    }

    pub fn emit_instr(&mut self, instr: Instr) -> CodeAddr {
        self.code.push(instr);
        (self.code.len() - 1) as CodeAddr
    }

    pub fn set_entry(&mut self, addr: CodeAddr) {
        self.entry = addr;
    }

    pub fn next_instr_addr(&self) -> CodeAddr {
        self.code.len() as CodeAddr
    }

    pub fn next_const_addr(&self) -> ConstAddr {
        self.consts.len() as ConstAddr
    }
}

impl fmt::Debug for Program {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "-- Data --")?;
        for (i, val) in self.consts.iter().enumerate() {
            writeln!(f, "{:X<4} | {:X?}", i, val)?;
        }
        writeln!(f, "-- Code --")?;
        for (i, instr) in self.code.iter().enumerate() {
            writeln!(f, "{:>#5X} | {:#X?}", i, instr)?;
        }
        Ok(())
    }
}