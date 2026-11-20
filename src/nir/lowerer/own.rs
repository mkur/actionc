//! Give an explicit OWN local in a reentrant routine private program-lifetime
//! storage. Keep the scoped LocalId as a view of that storage so all native
//! backends can use their normal data, descriptor and relocation paths.
use super::*;

pub(super) fn place_backings(
    routine: &str,
    locals: &mut [NirLocal],
    backings: &BTreeMap<LocalId, (SymbolId, Option<NirArrayGlobalFact>)>,
    globals: &mut Vec<NirGlobal>,
) {
    if backings.is_empty() {
        return;
    }
    // Resolve frame-relative initializer references before moving images
    // outside their routine. The verifier rejects references to automatic homes.
    let mut targets = BTreeMap::new();
    for local in locals.iter() {
        let target = match &local.backing {
            NirLocalBacking::Ordinary => backings.get(&local.id).map(|(id, _)| {
                (
                    NirDataAddressTarget::Storage(NirStorageId::Global(*id)),
                    0i64,
                )
            }),
            NirLocalBacking::Absolute(a) => Some((NirDataAddressTarget::Absolute(*a), 0)),
            NirLocalBacking::GlobalAlias { target, offset, .. } => Some((
                NirDataAddressTarget::Storage(NirStorageId::Global(*target)),
                i64::from(offset.get()),
            )),
            NirLocalBacking::Alias { target, offset, .. } => targets
                .get(target)
                .map(|&(base, addend)| (base, addend + i64::from(offset.get()))),
        };
        if let Some(target) = target {
            targets.insert(local.id, target);
        }
    }
    let resolve = |mut image: NirDataImage| {
        for fragment in &mut image.fragments {
            if let NirDataFragment::Address { target, addend, .. } = fragment
                && let NirDataAddressTarget::Storage(NirStorageId::Local(id)) = target
                && let Some(&(base, offset)) = targets.get(id)
            {
                *target = base;
                *addend += offset;
            }
        }
        image
    };
    for local in locals {
        let Some((id, array)) = backings.get(&local.id) else {
            continue;
        };
        let name = format!("{routine}::{}", local.name);
        let init = local.init.take().map(|init| match init {
            NirStorageInit::Bytes {
                image,
                zero_fill,
                mutable,
                section,
            } => NirGlobalInit::Bytes {
                image: resolve(image),
                zero_fill,
                mutable,
                section,
            },
            NirStorageInit::ZeroFill {
                bytes,
                mutable,
                section,
            } => NirGlobalInit::ZeroFill {
                bytes,
                mutable,
                section,
            },
            NirStorageInit::Descriptor {
                backing,
                descriptor_size,
                size_word,
                mutable,
                section,
            } => NirGlobalInit::Descriptor {
                backing: NirDataBacking {
                    owner: *id,
                    image: resolve(backing.image),
                    zero_fill: backing.zero_fill,
                    section: backing.section,
                },
                descriptor_size,
                size_word,
                mutable,
                section,
            },
        });
        local.backing = NirLocalBacking::GlobalAlias {
            target: *id,
            target_name: name.clone(),
            offset: ByteOffset::ZERO,
        };
        globals.push(NirGlobal {
            id: *id,
            name,
            kind: local.kind.clone(),
            ty: Some(local.ty.clone()),
            storage_size: local.layout.size,
            array: array.clone(),
            init,
            backing: NirGlobalBacking::Ordinary,
        });
    }
}
