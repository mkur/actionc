//! Target-owned legalization of a 32-bit NIR integer into two ordinary words.
//! No source typing or expression reassociation occurs here.
use super::*;

#[derive(Default)]
pub(super) struct WideValues {
    high: BTreeMap<TempId, MirTempId>,
}

pub(super) fn wide_type(ty: &NirType) -> bool {
    ty.kind.integer().is_some_and(|integer| integer.bits == 32)
}

impl WideValues {
    pub(super) fn new(routine: &NirRoutine, next: &mut u32, temps: &mut Vec<MirTemp>) -> Self {
        let high = routine
            .temps
            .iter()
            .filter(|temp| wide_type(&temp.ty))
            .map(|temp| (temp.id, generated_temp(next, temps)))
            .collect();
        Self { high }
    }

    pub(super) fn defs(&self, id: TempId) -> [MirDef; 2] {
        [
            MirDef::VTemp(MirTempId(id.0)),
            MirDef::VTemp(self.high[&id]),
        ]
    }

    pub(super) fn block_params(&self, param: &nir::NirBlockParam) -> Option<[MirBlockParam; 2]> {
        if !wide_type(&param.ty) {
            return None;
        }
        Some([
            MirBlockParam {
                dest: MirTempId(param.dest.0),
                width: MirWidth::Word,
            },
            MirBlockParam {
                dest: self.high[&param.dest],
                width: MirWidth::Word,
            },
        ])
    }

    pub(super) fn edge_values(&self, value: &NirValueKind) -> Option<[MirValue; 2]> {
        match value {
            NirValueKind::IntegerConst { bits, ty } if ty.bits == 32 => Some([
                MirValue::ConstU16(*bits as u16),
                MirValue::ConstU16((bits >> 16) as u16),
            ]),
            NirValueKind::Temp { id, ty } if wide_type(ty) => {
                Some([temp_value(MirTempId(id.0)), temp_value(self.high[id])])
            }
            _ => None,
        }
    }
}

pub(super) struct Builder<'a> {
    pub values: &'a WideValues,
    pub routine: &'a str,
    pub block: &'a str,
    pub next: &'a mut u32,
    pub temps: &'a mut Vec<MirTemp>,
    pub ops: &'a mut Vec<MirOp>,
    pub diagnostics: &'a mut Vec<MirDiagnostic>,
}

impl Builder<'_> {
    fn temp(&mut self) -> MirTempId {
        generated_temp(self.next, self.temps)
    }

    fn binary(
        &mut self,
        op: MirBinaryOp,
        left: MirValue,
        right: MirValue,
        width: MirWidth,
    ) -> MirValue {
        let dst = self.temp();
        self.ops.push(MirOp::Binary {
            op,
            dst: MirDef::VTemp(dst),
            left,
            right,
            width,
            carry_in: None,
            carry_out: MirCarryOut::Ignore,
        });
        temp_value(dst)
    }

    fn compare(
        &mut self,
        op: MirCompareOp,
        left: MirValue,
        right: MirValue,
        signed: bool,
    ) -> MirValue {
        let dst = self.temp();
        self.ops.push(MirOp::Compare {
            dst: MirCondDest::Temp(dst),
            op,
            left,
            right,
            width: MirWidth::Word,
            signed,
        });
        temp_value(dst)
    }

    fn widen_byte(&mut self, value: MirValue, signed: bool) -> MirValue {
        let dst = self.temp();
        self.ops.push(MirOp::Extend {
            dst: MirDef::VTemp(dst),
            src: value,
            from_width: MirWidth::Byte,
            to_width: MirWidth::Word,
            signed,
        });
        temp_value(dst)
    }

    pub(super) fn pair(
        &mut self,
        value: &NirValueKind,
        source_type: Option<&NirType>,
    ) -> Option<[MirValue; 2]> {
        if let Some(pair) = self.values.edge_values(value) {
            return Some(pair);
        }
        let signed = source_type.map(is_signed).unwrap_or_else(|| match value {
            NirValueKind::Temp { ty, .. } => is_signed(ty),
            NirValueKind::IntegerConst { ty, .. } => ty.signed,
            _ => false,
        });
        let width = source_type
            .and_then(mir_width)
            .or_else(|| value_width(value))?;
        let mut low = lower_value(self.routine, self.block, value, self.diagnostics)?;
        if width == MirWidth::Byte {
            low = self.widen_byte(low, signed);
        }
        let high = if signed {
            let sign = self.compare(MirCompareOp::Lt, low.clone(), MirValue::ConstU16(0), true);
            let sign = self.widen_byte(sign, false);
            self.binary(
                MirBinaryOp::Sub,
                MirValue::ConstU16(0),
                sign,
                MirWidth::Word,
            )
        } else {
            MirValue::ConstU16(0)
        };
        Some([low, high])
    }

    fn move_pair(&mut self, dest: TempId, pair: [MirValue; 2]) {
        for (dst, src) in self.values.defs(dest).into_iter().zip(pair) {
            self.ops.push(MirOp::Move {
                dst,
                src,
                width: MirWidth::Word,
            });
        }
    }

    fn address(
        &mut self,
        place: &NirPlace,
        addr_defs: &BTreeMap<TempId, MirAddrDef>,
    ) -> Option<MirAddr> {
        let address = lower_access_addr(
            self.routine,
            self.block,
            place,
            addr_defs,
            self.next,
            self.temps,
            self.ops,
            self.diagnostics,
        )?;
        if matches!(address, MirAddr::Direct(_)) {
            return Some(address);
        }
        // Capture once, including pointer-cell destinations which a first word
        // store might itself overwrite. Both halves use the same address value.
        let ptr = lower_computed_address_value(
            self.routine,
            self.block,
            address,
            place,
            self.next,
            self.temps,
            self.ops,
            self.diagnostics,
        )?;
        Some(MirAddr::Deref { ptr, offset: 0 })
    }

    fn add_sub(
        &mut self,
        op: MirBinaryOp,
        left: [MirValue; 2],
        right: [MirValue; 2],
    ) -> [MirValue; 2] {
        let [a, ah] = left;
        let [b, bh] = right;
        let low = self.binary(op, a.clone(), b.clone(), MirWidth::Word);
        // An explicit value transports carry/borrow between word operations;
        // word optimizers do not need to preserve a hidden flags dependency.
        let carry = self.compare(
            MirCompareOp::Lt,
            if op == MirBinaryOp::Add {
                low.clone()
            } else {
                a.clone()
            },
            if op == MirBinaryOp::Add { a } else { b },
            false,
        );
        let carry = self.widen_byte(carry, false);
        let high = self.binary(op, ah, bh, MirWidth::Word);
        let high = self.binary(op, high, carry, MirWidth::Word);
        [low, high]
    }

    fn compare_pair(
        &mut self,
        op: MirCompareOp,
        left: [MirValue; 2],
        right: [MirValue; 2],
        signed: bool,
    ) -> MirValue {
        let [a, ah] = left;
        let [b, bh] = right;
        let low = self.compare(op, a, b, false);
        let eq = self.compare(MirCompareOp::Eq, ah.clone(), bh.clone(), false);
        match op {
            MirCompareOp::Eq => self.binary(MirBinaryOp::And, eq, low, MirWidth::Byte),
            MirCompareOp::Ne => {
                let ne = self.binary(MirBinaryOp::Xor, eq, MirValue::ConstU8(1), MirWidth::Byte);
                self.binary(MirBinaryOp::Or, ne, low, MirWidth::Byte)
            }
            _ => {
                let high_op = if matches!(op, MirCompareOp::Lt | MirCompareOp::Le) {
                    MirCompareOp::Lt
                } else {
                    MirCompareOp::Gt
                };
                let high = self.compare(high_op, ah, bh, signed);
                let low = self.binary(MirBinaryOp::And, eq, low, MirWidth::Byte);
                self.binary(MirBinaryOp::Or, high, low, MirWidth::Byte)
            }
        }
    }

    pub(super) fn lower_op(
        &mut self,
        op: &NirOpKind,
        addr_defs: &BTreeMap<TempId, MirAddrDef>,
    ) -> bool {
        match op {
            NirOpKind::Load { dest, ty, place } | NirOpKind::VolatileLoad { dest, ty, place }
                if wide_type(ty) =>
            {
                if let Some(src) = self.address(place, addr_defs) {
                    let volatile = matches!(op, NirOpKind::VolatileLoad { .. });
                    if volatile {
                        self.ops.push(volatile_memory_barrier());
                    }
                    for (half, dst) in self.values.defs(*dest).into_iter().enumerate() {
                        self.ops.push(MirOp::Load {
                            dst,
                            src: offset_addr(&src, half as u16 * 2),
                            width: MirWidth::Word,
                        });
                    }
                    if volatile {
                        self.ops.push(volatile_memory_barrier());
                    }
                }
            }
            NirOpKind::Store { src, ty, place } | NirOpKind::VolatileStore { src, ty, place }
                if wide_type(ty) =>
            {
                if let Some(dst) = self.address(place, addr_defs)
                    && let Some(pair) = self.pair(src, None)
                {
                    let volatile = matches!(op, NirOpKind::VolatileStore { .. });
                    if volatile {
                        self.ops.push(volatile_memory_barrier());
                    }
                    for (half, src) in pair.into_iter().enumerate() {
                        self.ops.push(MirOp::Store {
                            dst: offset_addr(&dst, half as u16 * 2),
                            src,
                            width: MirWidth::Word,
                        });
                    }
                    if volatile {
                        self.ops.push(volatile_memory_barrier());
                    }
                }
            }
            NirOpKind::Cast {
                dest,
                src,
                from,
                to,
                ..
            } if wide_type(from) || wide_type(to) => {
                if let Some(pair) = self.pair(src, Some(from)) {
                    if wide_type(to) {
                        self.move_pair(*dest, pair);
                    } else if let Some(width) = mir_width(to) {
                        let dst = MirDef::VTemp(MirTempId(dest.0));
                        if width == MirWidth::Word {
                            self.ops.push(MirOp::Move {
                                dst,
                                src: pair[0].clone(),
                                width,
                            });
                        } else {
                            self.ops.push(MirOp::Truncate {
                                dst,
                                src: pair[0].clone(),
                                from_width: MirWidth::Word,
                                to_width: width,
                            });
                        }
                    } else {
                        self.unsupported("cast");
                    }
                }
            }
            NirOpKind::Unary { dest, ty, op, src } if wide_type(ty) => {
                if let Some(pair) = self.pair(src, None) {
                    let pair = if *op == NirUnaryOp::Neg {
                        self.add_sub(
                            MirBinaryOp::Sub,
                            [MirValue::ConstU16(0), MirValue::ConstU16(0)],
                            pair,
                        )
                    } else {
                        pair
                    };
                    self.move_pair(*dest, pair);
                }
            }
            NirOpKind::Binary {
                dest,
                ty,
                op,
                left,
                right,
            } if wide_type(ty) => {
                if let Some(left) = self.pair(left, None)
                    && let Some(right) = self.pair(right, None)
                {
                    let pair = match op {
                        NirBinaryOp::Add | NirBinaryOp::Sub => {
                            self.add_sub(mir_binary_op(*op), left, right)
                        }
                        NirBinaryOp::And | NirBinaryOp::Or | NirBinaryOp::Xor => [
                            self.binary(
                                mir_binary_op(*op),
                                left[0].clone(),
                                right[0].clone(),
                                MirWidth::Word,
                            ),
                            self.binary(
                                mir_binary_op(*op),
                                left[1].clone(),
                                right[1].clone(),
                                MirWidth::Word,
                            ),
                        ],
                        _ => {
                            self.unsupported(&format!("{op:?}"));
                            return true;
                        }
                    };
                    self.move_pair(*dest, pair);
                }
            }
            NirOpKind::Compare {
                dest,
                operand_ty,
                op,
                left,
                right,
                ..
            } if wide_type(operand_ty) => {
                if let Some(left) = self.pair(left, None)
                    && let Some(right) = self.pair(right, None)
                {
                    let src =
                        self.compare_pair(mir_compare_op(*op), left, right, is_signed(operand_ty));
                    self.ops.push(MirOp::Move {
                        dst: MirDef::VTemp(MirTempId(dest.0)),
                        src,
                        width: MirWidth::Byte,
                    });
                }
            }
            _ => return false,
        }
        true
    }

    fn unsupported(&mut self, operation: &str) {
        self.diagnostics.push(MirDiagnostic::block(
            self.routine,
            self.block,
            format!("MIR6502 32-bit {operation} is not implemented yet"),
        ));
    }
}
