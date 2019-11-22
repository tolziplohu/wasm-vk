use crate::*;
use rspirv::dr;
use spirv_headers as spvh;

trait ToSpirv {
    type Value;
    type Ctx;
    fn spv(self, ctx: &mut Self::Ctx) -> Self::Value;
}

#[derive(Default)]
struct Types {
    b: Option<u32>,
    i_32: Option<u32>,
    i_64: Option<u32>,
    f_32: Option<u32>,
    f_64: Option<u32>,
}

use std::collections::HashMap;

#[derive(Debug, Copy, Clone)]
struct Loop {
    head: u32,
    cont: u32,
    end: u32,
}

#[derive(Default)]
pub struct Ctx {
    tys: Types,
    ptrs: HashMap<(wasm::ValueType, spvh::StorageClass), u32>,
    buffer: u32,
    /// The x component of the thread id - unique to a function
    thread_id: u32,
    /// The thread id uvec3 - global
    thread_id_v3: u32,
    locals: IndexMap<u32>,
    b: dr::Builder,
    /// (Function, type)
    funs: Vec<(u32, u32)>,
    loops: Vec<Loop>,
}
impl Ctx {
    pub fn new() -> Self {
        let mut b = dr::Builder::new();

        b.set_version(1, 0);
        b.capability(spvh::Capability::Shader);
        b.ext_inst_import("GLSL.std.450");
        b.memory_model(spvh::AddressingModel::Logical, spvh::MemoryModel::GLSL450);

        // A temporary context mostly so we can use the type cache
        let mut c = Ctx {
            tys: Default::default(),
            ptrs: Default::default(),
            buffer: 0,
            thread_id: 0,
            thread_id_v3: 0,
            locals: IndexMap::default(),
            b,
            funs: Vec::new(),
            loops: Vec::new(),
        };

        let t_uint = c.get(wasm::ValueType::I32);
        let t_arr = c.type_runtime_array(t_uint);
        let t_struct = c.type_struct([t_arr]);
        let t_ptr = c.type_pointer(None, spvh::StorageClass::Uniform, t_struct);
        let buffer = c.variable(t_ptr, None, spvh::StorageClass::Uniform, None);

        // This is deprecated past SPIR-V 1.3, and should be replaced with the StorageBuffer StorageClass.
        // I don't know that any Vulkan implementations actually support that yet, though, so this works for now.
        c.decorate(t_struct, spvh::Decoration::BufferBlock, []);
        // Set 0, binding 0
        c.decorate(
            buffer,
            spvh::Decoration::DescriptorSet,
            [dr::Operand::LiteralInt32(0)],
        );
        c.decorate(
            buffer,
            spvh::Decoration::Binding,
            [dr::Operand::LiteralInt32(0)],
        );
        c.decorate(
            t_arr,
            spvh::Decoration::ArrayStride,
            [dr::Operand::LiteralInt32(4)],
        );
        c.member_decorate(
            t_struct,
            0,
            spvh::Decoration::Offset,
            [dr::Operand::LiteralInt32(0)],
        );

        let t_uvec3 = c.type_vector(t_uint, 3);
        let t_uvec3_ptr = c.type_pointer(None, spvh::StorageClass::Input, t_uvec3);
        let thread_id_v3 = c.variable(t_uvec3_ptr, None, spvh::StorageClass::Input, None);
        c.decorate(
            thread_id_v3,
            spvh::Decoration::BuiltIn,
            [dr::Operand::BuiltIn(spvh::BuiltIn::GlobalInvocationId)],
        );

        Ctx {
            buffer,
            thread_id_v3,
            ..c
        }
    }

    pub fn fun(&mut self, f: ir::Fun<ir::Base>) {
        let ir::Fun { params, body, ty } = f;
        let locals = body.locals();

        let ret = if let Some(ty) = ty {
            self.get(ty)
        } else {
            self.type_void()
        };
        let param_tys: Vec<_> = params.iter().map(|x| self.get(*x)).collect();
        let t = self.type_function(ret, param_tys);
        let fun = self
            .begin_function(ret, None, spvh::FunctionControl::NONE, t)
            .unwrap();
        self.begin_basic_block(None).unwrap();

        let mut max = 0;
        let mut locals_m = IndexMap::with_capacity(locals.len());
        for l in locals {
            let ty = l.ty;
            let ty = self.ptr(ty, spvh::StorageClass::Function);
            let n = self.variable(ty, None, spvh::StorageClass::Function, None);
            locals_m.insert(l.idx, n);
            max = l.idx.max(max);
        }

        // Parameters are separate from locals in SPIR-V, so we store them into the corresponding locals
        for (idx, ty) in params.into_iter().enumerate() {
            let ty = self.get(ty);
            let n = self.function_parameter(ty).unwrap();
            // If it's not in locals_m, it's not used - no need to store it anywhere
            if let Some(l) = locals_m.get(idx as u32) {
                self.store(*l, n, None, []).unwrap();
            }
        }

        self.locals = locals_m;

        // We need to initialize `self.thread_id` from `self.thread_id_v3`
        let t_uint = self.get(wasm::ValueType::I32);
        let t_uint_ptr = self.type_pointer(None, spvh::StorageClass::Input, t_uint);
        let const_0 = self.constant_u32(t_uint, 0);
        let thread_id_v3 = self.thread_id_v3;
        let thread_id = self
            .access_chain(t_uint_ptr, None, thread_id_v3, [const_0])
            .unwrap();
        let thread_id = self.load(t_uint, None, thread_id, None, []).unwrap();
        self.thread_id = thread_id;

        // Now compile the body
        let r = body.spv(self);
        if ty.is_some() {
            self.ret_value(r).unwrap();
        } else {
            self.ret().unwrap();
        }
        self.end_function().unwrap();

        self.funs.push((fun, ret));
    }

    pub fn finish(mut self, entry: Option<u32>) -> dr::Module {
        if let Some(entry) = entry {
            let (entry, _) = self.funs[entry as usize];

            let id = self.thread_id_v3;
            self.entry_point(spvh::ExecutionModel::GLCompute, entry, "main", [id]);
            self.execution_mode(entry, spvh::ExecutionMode::LocalSize, [64, 1, 1]);
        }
        self.b.module()
    }

    fn bool(&mut self) -> u32 {
        if let Some(i) = self.tys.b {
            i
        } else {
            let i = self.type_bool();
            self.tys.b = Some(i);
            i
        }
    }

    fn int(&mut self, width: ir::Width) -> u32 {
        match width {
            ir::Width::W32 => self.get(wasm::ValueType::I32),
            ir::Width::W64 => self.get(wasm::ValueType::I64),
        }
    }

    fn ptr(&mut self, t: wasm::ValueType, class: spvh::StorageClass) -> u32 {
        if let Some(i) = self.ptrs.get(&(t, class)) {
            *i
        } else {
            let i = self.get(t);
            let i = self.type_pointer(None, class, i);
            self.ptrs.insert((t, class), i);
            i
        }
    }

    fn get(&mut self, t: wasm::ValueType) -> u32 {
        match t {
            wasm::ValueType::I32 => {
                if let Some(i) = self.tys.i_32 {
                    i
                } else {
                    let i = self.type_int(32, 0);
                    self.tys.i_32 = Some(i);
                    i
                }
            }
            wasm::ValueType::I64 => {
                if let Some(i) = self.tys.i_64 {
                    i
                } else {
                    let i = self.type_int(64, 0);
                    self.tys.i_64 = Some(i);
                    i
                }
            }
            wasm::ValueType::F32 => {
                if let Some(i) = self.tys.f_32 {
                    i
                } else {
                    let i = self.type_float(32);
                    self.tys.f_32 = Some(i);
                    i
                }
            }
            wasm::ValueType::F64 => {
                if let Some(i) = self.tys.f_64 {
                    i
                } else {
                    let i = self.type_float(64);
                    self.tys.f_64 = Some(i);
                    i
                }
            }
        }
    }
}
use std::ops::{Deref, DerefMut};
impl Deref for Ctx {
    type Target = dr::Builder;
    fn deref(&self) -> &dr::Builder {
        &self.b
    }
}
impl DerefMut for Ctx {
    fn deref_mut(&mut self) -> &mut dr::Builder {
        &mut self.b
    }
}

impl ToSpirv for ir::Base {
    type Ctx = Ctx;
    type Value = u32;
    fn spv(self, ctx: &mut Ctx) -> u32 {
        match self {
            ir::Base::Call(i, params) => {
                let (f, t) = ctx.funs[i as usize];
                let params: Vec<_> = params.into_iter().map(|x| x.spv(ctx)).collect();
                ctx.function_call(t, None, f, params).unwrap()
            }
            ir::Base::Nop => 0,
            ir::Base::INumOp(w, op, a, b) => {
                let a = a.spv(ctx);
                let b = b.spv(ctx);
                let ty = ctx.int(w);
                match op {
                    ir::INumOp::Mul => ctx.i_mul(ty, None, a, b).unwrap(),
                    ir::INumOp::Add => ctx.i_add(ty, None, a, b).unwrap(),
                    ir::INumOp::Sub => ctx.i_sub(ty, None, a, b).unwrap(),
                    ir::INumOp::Shl => ctx.shift_left_logical(ty, None, a, b).unwrap(),
                    ir::INumOp::ShrS => ctx.shift_right_arithmetic(ty, None, a, b).unwrap(),
                    ir::INumOp::ShrU => ctx.shift_right_logical(ty, None, a, b).unwrap(),
                    ir::INumOp::DivU => ctx.u_div(ty, None, a, b).unwrap(),
                    ir::INumOp::DivS => ctx.s_div(ty, None, a, b).unwrap(),
                }
            }
            ir::Base::ICompOp(w, op, a, b) => {
                let a = a.spv(ctx);
                let b = b.spv(ctx);
                let ty = ctx.int(w);
                // Unlike WASM, SPIR-V has booleans
                // So we convert them to integers immediately
                let t_bool = ctx.bool();

                let b = match op {
                    ir::ICompOp::Eq => ctx.i_equal(t_bool, None, a, b).unwrap(),
                    ir::ICompOp::NEq => ctx.i_not_equal(t_bool, None, a, b).unwrap(),
                    ir::ICompOp::LeU => ctx.u_less_than_equal(t_bool, None, a, b).unwrap(),
                    ir::ICompOp::GeU => ctx.u_greater_than_equal(t_bool, None, a, b).unwrap(),
                    ir::ICompOp::LtU => ctx.u_less_than(t_bool, None, a, b).unwrap(),
                    ir::ICompOp::GtU => ctx.u_greater_than(t_bool, None, a, b).unwrap(),

                    ir::ICompOp::LeS => ctx.s_less_than_equal(t_bool, None, a, b).unwrap(),
                    ir::ICompOp::GeS => ctx.s_greater_than_equal(t_bool, None, a, b).unwrap(),
                    ir::ICompOp::LtS => ctx.s_less_than(t_bool, None, a, b).unwrap(),
                    ir::ICompOp::GtS => ctx.s_greater_than(t_bool, None, a, b).unwrap(),
                };

                let zero = ctx.constant_u32(ty, 0);
                let one = ctx.constant_u32(ty, 1);
                ctx.select(ty, None, b, one, zero).unwrap()
            }
            ir::Base::Const(ir::Const::I32(i)) => {
                let ty = ctx.get(wasm::ValueType::I32);
                ctx.constant_u32(ty, unsafe { std::mem::transmute(i) })
            }
            ir::Base::Const(ir::Const::F32(i)) => {
                let ty = ctx.get(wasm::ValueType::F32);
                ctx.constant_f32(ty, i)
            }
            ir::Base::Const(_) => panic!("We currently don't support 64-bit constants"),
            ir::Base::Seq(a, b) => {
                a.spv(ctx);
                b.spv(ctx)
            }
            ir::Base::GetLocal(l) => {
                let ty = ctx.get(l.ty);
                let l = *ctx.locals.get(l.idx).unwrap();
                ctx.load(ty, None, l, None, []).unwrap()
            }
            ir::Base::SetLocal(l, val) => {
                let l = *ctx.locals.get(l.idx).unwrap();
                let val = val.spv(ctx);
                ctx.store(l, val, None, []).unwrap();
                0
            }
            ir::Base::GetGlobal(g) => {
                assert_eq!(
                    g,
                    ir::Global {
                        ty: wasm::GlobalType::new(wasm::ValueType::I32, false),
                        idx: 0
                    }
                );
                ctx.thread_id
            }
            ir::Base::Load(ty, ptr) => {
                let uint = ctx.get(wasm::ValueType::I32);
                let c0 = ctx.constant_u32(uint, 0);

                let ptr_ty = ctx.ptr(ty, spvh::StorageClass::Uniform);
                let ty = ctx.get(ty);
                let ptr = ptr.spv(ctx);
                // Divide by four because of the size of a u32
                let c4 = ctx.constant_u32(uint, 4);
                let ptr = ctx.u_div(uint, None, ptr, c4).unwrap();

                let buf = ctx.buffer;
                let ptr = ctx.access_chain(ptr_ty, None, buf, [c0, ptr]).unwrap();
                ctx.load(ty, None, ptr, None, []).unwrap()
            }
            ir::Base::Store(ty, ptr, val) => {
                let uint = ctx.get(wasm::ValueType::I32);
                let c0 = ctx.constant_u32(uint, 0);

                // The pointer is lower in the stack for the WASM store instruction, so it gets evaluated first.
                let ptr = ptr.spv(ctx);
                let val = val.spv(ctx);

                let ptr_ty = ctx.ptr(ty, spvh::StorageClass::Uniform);
                // Divide by four because of the size of a u32
                let c4 = ctx.constant_u32(uint, 4);
                let ptr = ctx.u_div(uint, None, ptr, c4).unwrap();

                let buf = ctx.buffer;
                let ptr = ctx.access_chain(ptr_ty, None, buf, [c0, ptr]).unwrap();
                ctx.store(ptr, val, None, []).unwrap();
                0
            }
            ir::Base::If { cond, t, f } => {
                let l_t = ctx.id();
                let l_f = ctx.id();
                let l_m = ctx.id();

                let cond = cond.spv(ctx);
                // `cond` is a number, so to turn it into a boolean we do `cond != 0`
                let t_uint = ctx.get(wasm::ValueType::I32);
                let c0 = ctx.constant_u32(t_uint, 0);
                let t_bool = ctx.bool();
                let cond = ctx.i_not_equal(t_bool, None, cond, c0).unwrap();

                ctx.selection_merge(l_m, spvh::SelectionControl::NONE)
                    .unwrap();
                ctx.branch_conditional(cond, l_t, l_f, []).unwrap();
                ctx.begin_basic_block(Some(l_t)).unwrap();
                t.spv(ctx);
                ctx.branch(l_m).unwrap();
                ctx.begin_basic_block(Some(l_f)).unwrap();
                f.spv(ctx);
                ctx.branch(l_m).unwrap();
                ctx.begin_basic_block(Some(l_m)).unwrap();

                0
            }
            ir::Base::Loop(a) => {
                let head = ctx.id();
                let cont = ctx.id();
                let end = ctx.id();
                let body = ctx.id();

                ctx.branch(head).unwrap();
                ctx.begin_basic_block(Some(head)).unwrap();
                ctx.loop_merge(end, cont, spvh::LoopControl::NONE, [])
                    .unwrap();
                ctx.branch(body).unwrap();
                ctx.begin_basic_block(Some(body)).unwrap();

                ctx.loops.push(Loop { head, cont, end });

                a.spv(ctx);

                ctx.loops.pop().unwrap();

                // For WASM loops, the default behaviour is to break out of a loop at the end
                ctx.branch(end).unwrap();

                // SPIR-V requires the continue block to be after the rest of the loop
                ctx.begin_basic_block(Some(cont)).unwrap();
                ctx.branch(head).unwrap();

                ctx.begin_basic_block(Some(end)).unwrap();

                0
            }
            ir::Base::Continue => {
                let l = *ctx.loops.last().unwrap();
                ctx.branch(l.cont).unwrap();
                ctx.begin_basic_block(None).unwrap();
                0
            }
            ir::Base::Break => {
                let l = *ctx.loops.last().unwrap();
                ctx.branch(l.end).unwrap();
                ctx.begin_basic_block(None).unwrap();
                0
            }
            ir::Base::Return => {
                ctx.ret().unwrap();
                // Unreacheable block
                ctx.begin_basic_block(None).unwrap();
                0
            }
        }
    }
}

pub fn module_bytes(m: dr::Module) -> Vec<u8> {
    use rspirv::binary::Assemble;

    let mut spv = m.assemble();
    // TODO: test this on a big-endian system
    for i in spv.iter_mut() {
        *i = i.to_le()
    }
    let spv: &[u8] = unsafe { spv.align_to().1 };
    spv.to_vec()
}
