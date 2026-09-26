//! Bounded incoming-parameter read/capture forwarding. No cross-call residence.
use super::*;

impl Builder<'_> {
    fn incoming_word(&self, address: &Mir65816Address) -> Result<Option<(ParamId, Slot)>, String> {
        let Mir65816AddressBase::Parameter(id) = address.base else {
            return Ok(None);
        };
        if address.index.is_some() || address.displacement.get() != 0 || self.code.delta() != 0 {
            return Ok(None);
        }
        let p = self
            .routine
            .frame
            .parameters
            .iter()
            .find(|p| p.param == id)
            .ok_or("unknown parameter")?;
        if p.frame_object.is_some()
            || !matches!(p.incoming,Mir65816AbiHome::StackArgument{size,..} if size.get()==2)
            || self
                .routine
                .frame
                .objects
                .iter()
                .any(|o| o.owner == Mir65816FrameObjectOwner::Param(id))
        {
            return Ok(None);
        }
        // Layout verification alone does not certify every operation's effects.
        // Refuse escaped, written or noncanonical parameter addresses even in
        // hand-mutated MIR whose home metadata still claims immutability.
        let refers = |a: &Mir65816Address| a.base == Mir65816AddressBase::Parameter(id);
        let unsafe_use = self
            .routine
            .blocks
            .iter()
            .flat_map(|b| &b.ops)
            .any(|op| match op {
                Mir65816Op::Load {
                    address,
                    width,
                    volatile,
                    ..
                } => {
                    refers(address)
                        && (*volatile
                            || width.get() != 2
                            || address.index.is_some()
                            || address.displacement.get() != 0)
                }
                Mir65816Op::Store { address, .. } | Mir65816Op::AddressOf { address, .. } => {
                    refers(address)
                }
                Mir65816Op::Copy {
                    source,
                    destination,
                    ..
                } => refers(source) || refers(destination),
                _ => false,
            });
        if unsafe_use {
            return Ok(None);
        }
        let offset = self.word_displacement(self.incoming(id)?)?;
        Ok(Some((
            id,
            Slot {
                offset: offset.into(),
                width: 2,
            },
        )))
    }
    pub(super) fn incoming_word_load(
        &mut self,
        dest: TempId,
        bytes: u8,
        address: &Mir65816Address,
        volatile: bool,
    ) -> Result<bool, String> {
        if volatile || bytes != 2 {
            return Ok(false);
        }
        let Some((id, source)) = self.incoming_word(address)? else {
            return Ok(false);
        };
        let capture = self.temp(dest)?;
        if capture.slot().width != 2 {
            return Err("load temporary width mismatch".into());
        }
        word_home(capture, self.code.delta())?;
        if capture.overlaps(Location::Stack(source)) {
            return Err("parameter capture overlaps incoming word".into());
        }
        self.code.capture_incoming_word(id, source, dest, capture);
        Ok(true)
    }
}

impl Builder<'_> {
    pub(super) fn incoming_comparisons(
        &self,
        block: &Mir65816Block,
        counts: &BTreeMap<TempId, usize>,
    ) -> Result<BTreeMap<usize, Condition>, String> {
        let mut out = BTreeMap::new();
        for (i, pair) in block.ops.windows(2).enumerate() {
            let Mir65816Op::Load {
                dest: loaded,
                width,
                address,
                volatile: false,
            } = &pair[0]
            else {
                continue;
            };
            if width.get() != 2 || counts.get(loaded) != Some(&1) {
                continue;
            }
            let Some((id, incoming)) = self.incoming_word(address)? else {
                continue;
            };
            let capture = self.temp(*loaded)?;
            if capture.slot().width != 2 {
                return Err("incoming compare capture width mismatch".into());
            }
            word_home(capture, self.code.delta())?;
            if capture.overlaps(Location::Stack(incoming)) {
                return Err("incoming compare capture overlaps parameter".into());
            }
            let Mir65816Op::Compare {
                dest,
                width,
                signed: false,
                operation,
                left,
                right,
            } = &pair[1]
            else {
                continue;
            };
            if width.get() != 2 {
                continue;
            }
            // Preflight the original definition/consumer before substituting
            // the incoming home. No executable operation can intervene.
            if self
                .condition(*dest, 2, false, *operation, left, right)?
                .is_none()
            {
                continue;
            }
            let is_loaded =
                |v: &Mir65816Value| matches!(v,Mir65816Value::Temp(t,w) if t==loaded && w.get()==2);
            let parameter = Mir65816Value::Param(id);
            let (left, right) = if is_loaded(left) {
                (&parameter, right)
            } else if is_loaded(right) {
                (left, &parameter)
            } else {
                continue;
            };
            if self.temp(*dest)?.overlaps(Location::Stack(incoming)) {
                return Err("incoming compare result overlaps parameter".into());
            }
            if let Some(condition) = self.condition(*dest, 2, false, *operation, left, right)? {
                out.insert(i + 1, condition);
            }
        }
        Ok(out)
    }
}
