//! Narrow checked X/home relation. CFG obligations authorize joins, never a
//! fallthrough observation or the fact that X happened to contain a value.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XContract {
    pub param: TempId,
    pub home: Location,
    pub increment: Option<(TempId, Location)>,
    pub header: Label,
    pub body: Label,
    pub predecessors: BTreeSet<Label>,
}

impl TrackedEmitter65816 {
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn prove_x(&mut self, contract: XContract) {
        self.request(Request::ProveX(contract.clone()), |this| {
        assert!(this.x_contract.is_none() && this.blocks.is_disjoint(&this.bound));
        assert!(
            matches!(contract.home, Location::DirectPage(s) if s.width == 2 && super::super::scalar::word_offset(s.offset))
        );
        if let Some((update, home)) = contract.increment {
            assert_ne!(update, contract.param);
            assert!(!home.overlaps(contract.home));
            assert!(
                matches!(home,Location::DirectPage(s) if s.width==2 && super::super::scalar::word_offset(s.offset))
            );
        }
        assert_ne!(contract.header, contract.body);
        assert_eq!(contract.predecessors.len(), 2);
        assert!(contract.predecessors.contains(&contract.body));
        let edges = this.remaining_edges.as_ref().expect("X needs checked CFG");
        assert_eq!(
            edges[&contract.header],
            contract
                .predecessors
                .iter()
                .map(|&l| (Some(l), 1))
                .collect()
        );
        assert_eq!(edges[&contract.body], [(Some(contract.header), 1)].into());
        for label in [contract.header, contract.body] {
            assert!(this.proved_blocks.contains(&label));
            this.entries.get_mut(&label).unwrap().x_word = true;
        }
        this.x_contract = Some(contract);
            })
    }

    pub(super) fn x_edge(&mut self, label: Label) -> bool {
        assert!(
            !self.x_reserved || self.x_valid,
            "pending X relation at control-flow boundary"
        );
        if self.blocks.contains(&label) {
            if let Some(c) = &self.x_contract {
                if label == c.header || label == c.body {
                    assert!(self.x_valid, "missing X predecessor relation");
                    if label == c.header {
                        assert!(self.x_refreshed, "missing X edge refresh");
                        self.x_refreshed = false;
                    }
                    self.require_x();
                    return true;
                }
            }
            false
        } else {
            self.x_valid
        }
    }

    pub(super) fn x_join(&mut self, live: bool) {
        self.x_reserved = live;
        self.x_valid = live;
        self.x_refreshed = false;
        if live {
            let home = self.x_contract.as_ref().expect("X entry contract").home;
            let value = self.state.fresh(Width::Word);
            self.state.x = value;
            self.state.bind_home(home, value);
        }
    }

    fn require_x(&self) {
        assert!(self.x_reserved && self.x_valid, "stale X relation");
        assert_eq!(self.state.env.index, Width::Word);
        let c = self.x_contract.as_ref().unwrap();
        assert!(
            self.state
                .homes
                .get(&c.home)
                .is_some_and(|h| h.value.matches(self.state.x)),
            "X/home identity mismatch"
        );
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn refresh_x(&mut self) {
        self.request(Request::RefreshX, |this| {
            let c = this.x_contract.as_ref().expect("unplanned X refresh");
            assert!(
                c.predecessors
                    .contains(&this.active_block.expect("X edge source"))
            );
            assert!(!this.x_refreshed, "duplicate X refresh");
            assert_eq!(
                (this.state.env.m, this.state.env.index),
                (Width::Word, Width::Word)
            );
            let at = this.position().checked_sub(2).expect("missing X copy tail");
            let offset = c.home.slot().offset as u8;
            assert!(this.code.boundaries.contains(&at));
            assert!(
                this.code.bytes[at..] == [0x85, offset] || this.code.bytes[at..] == [0xa5, offset],
                "X refresh needs final home store/read"
            );
            assert!(
                this.state.a.width() == Some(Width::Word) && this.state.nz.matches(this.state.a)
            );
            let home = c.home;
            this.x_access = true;
            this.op(Implied::Tax);
            this.x_access = false;
            // A real checked read or retained store supplies this observation.
            this.state.bind_home(home, this.state.x);
            this.x_reserved = true;
            this.x_valid = true;
            this.x_refreshed = true;
            this.observe();
        })
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn load_x_word(&mut self, param: Option<TempId>, home: Option<Location>) -> bool {
        self.request(Request::LoadX(param, home), |this| {
            if !this.x_reserved
                || !this
                    .x_contract
                    .as_ref()
                    .is_some_and(|c| Some(c.param) == param && Some(c.home) == home)
            {
                return false;
            }
            this.require_x();
            assert_eq!(this.state.env.m, Width::Word);
            this.x_access = true;
            this.op(Implied::Txa);
            this.x_access = false;
            true
        })
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn compare_x_word(&mut self, param: TempId, home: Location, threshold: u16) {
        self.request(Request::CompareX(param, home, threshold), |this| {
            let c = this.x_contract.as_ref().expect("unplanned X compare");
            assert_eq!((param, home), (c.param, c.home));
            this.require_x();
            this.x_access = true;
            this.word(WordOp::CpxImm, threshold);
            this.x_access = false;
        })
    }

    /// The checked ADD's only MIR output is its word value; C/V are dead.
    /// Keep X reserved while it contains q and the authoritative p home is old.
    pub fn increment_x_word(
        &mut self,
        param: TempId,
        home: Location,
        update: TempId,
        destination: Location,
    ) {
        self.request(
            Request::IncrementX(param, home, update, destination),
            |this| {
                let c = this.x_contract.as_ref().expect("unplanned X increment");
                assert_eq!((param, home), (c.param, c.home));
                assert_eq!(c.increment, Some((update, destination)));
                assert_eq!(this.active_block, Some(c.body));
                this.require_x();
                assert_eq!(this.state.env.m, Width::Word);
                this.x_valid = false;
                this.x_access = true;
                this.op(Implied::Inx);
                this.op(Implied::Txa);
                this.x_access = false;
            },
        )
    }

    pub(super) fn x_implied(&self, op: Implied) {
        if self.x_reserved {
            assert!(
                matches!(op, Implied::Clc | Implied::Sec)
                    || self.x_access && matches!(op, Implied::Tax | Implied::Txa | Implied::Inx),
                "instruction clobbers X reservation"
            );
        }
    }
    pub(super) fn x_byte(&mut self, op: ByteOp, value: u8) {
        if !self.x_reserved {
            return;
        }
        use ByteOp::*;
        assert!(
            matches!(
                op,
                LdaImm
                    | AdcImm
                    | SbcImm
                    | CmpImm
                    | EorImm
                    | LdaStack
                    | StaStack
                    | AdcStack
                    | SbcStack
                    | CmpStack
                    | LdaDp
                    | StaDp
                    | AdcDp
                    | SbcDp
                    | CmpDp
            ) || matches!(op, Rep | Sep) && value == 0x20,
            "instruction clobbers X reservation"
        );
        if op == StaDp {
            let store = Location::DirectPage(Slot {
                offset: value.into(),
                width: self.state.env.m.bytes(),
            });
            if store.overlaps(self.x_contract.as_ref().unwrap().home) {
                self.x_valid = false;
            }
        }
    }
}
