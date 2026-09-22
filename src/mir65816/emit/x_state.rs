//! Narrow checked X/home relation. CFG obligations authorize joins, never a
//! fallthrough observation or the fact that X happened to contain a value.
use super::*;

#[derive(Clone, Debug)]
pub struct XContract {
    pub param: TempId,
    pub home: Location,
    pub header: Label,
    pub body: Label,
    pub predecessors: BTreeSet<Label>,
}

impl TrackedEmitter65816 {
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn prove_x(&mut self, contract: XContract) {
        assert!(self.x_contract.is_none() && self.blocks.is_disjoint(&self.bound));
        assert!(
            matches!(contract.home, Location::DirectPage(s) if s.width == 2 && super::super::scalar::word_offset(s.offset))
        );
        assert_ne!(contract.header, contract.body);
        assert_eq!(contract.predecessors.len(), 2);
        assert!(contract.predecessors.contains(&contract.body));
        let edges = self.remaining_edges.as_ref().expect("X needs checked CFG");
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
            assert!(self.proved_blocks.contains(&label));
            self.entries.get_mut(&label).unwrap().x_word = true;
        }
        self.x_contract = Some(contract);
    }

    pub(super) fn x_edge(&mut self, label: Label) -> bool {
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
        let c = self.x_contract.as_ref().expect("unplanned X refresh");
        assert!(
            c.predecessors
                .contains(&self.active_block.expect("X edge source"))
        );
        assert!(!self.x_refreshed, "duplicate X refresh");
        assert_eq!(
            (self.state.env.m, self.state.env.index),
            (Width::Word, Width::Word)
        );
        let at = self.position().checked_sub(2).expect("missing X copy tail");
        let offset = c.home.slot().offset as u8;
        assert!(self.code.boundaries.contains(&at));
        assert!(
            self.code.bytes[at..] == [0x85, offset] || self.code.bytes[at..] == [0xa5, offset],
            "X refresh needs final home store/read"
        );
        assert!(self.state.a.width() == Some(Width::Word) && self.state.nz.matches(self.state.a));
        let home = c.home;
        self.x_access = true;
        self.op(Implied::Tax);
        self.x_access = false;
        // A real checked read or retained store supplies this observation.
        self.state.bind_home(home, self.state.x);
        self.x_reserved = true;
        self.x_valid = true;
        self.x_refreshed = true;
        self.observe();
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn load_x_word(&mut self, param: Option<TempId>, home: Option<Location>) -> bool {
        if !self.x_reserved
            || !self
                .x_contract
                .as_ref()
                .is_some_and(|c| Some(c.param) == param && Some(c.home) == home)
        {
            return false;
        }
        self.require_x();
        assert_eq!(self.state.env.m, Width::Word);
        self.x_access = true;
        self.op(Implied::Txa);
        self.x_access = false;
        true
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn compare_x_word(&mut self, param: TempId, home: Location, threshold: u16) {
        let c = self.x_contract.as_ref().expect("unplanned X compare");
        assert_eq!((param, home), (c.param, c.home));
        self.require_x();
        self.x_access = true;
        self.word(WordOp::CpxImm, threshold);
        self.x_access = false;
    }

    pub(super) fn x_implied(&self, op: Implied) {
        if self.x_reserved {
            assert!(
                matches!(op, Implied::Clc | Implied::Sec)
                    || self.x_access && matches!(op, Implied::Tax | Implied::Txa),
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
