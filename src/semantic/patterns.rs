//! Typed patterns and bounded constructor/product usefulness. Scalar domains
//! stay open: literals refine a match, but only a wildcard closes that domain.
use super::variants::VariantConstructorId;
use super::*;

pub(super) const MAX_PATTERN_DEPTH: usize = 64;
const MAX_COVERAGE_STEPS: usize = 262_144;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum Pattern {
    Wild,
    Constructor(VariantConstructorId, Vec<Pattern>),
    Literal(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Domain {
    Variant(SymbolId),
    Open,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Query {
    matrix: Vec<Vec<Pattern>>,
    row: Vec<Pattern>,
    domains: Vec<Domain>,
}

pub(super) struct Coverage<'a> {
    types: &'a HashMap<SymbolId, super::variants::VariantType>,
    fields: &'a [SemanticField],
    memo: HashMap<Query, Option<Vec<Pattern>>>,
    remaining: usize,
}

impl<'a> Coverage<'a> {
    pub(super) fn new(
        types: &'a HashMap<SymbolId, super::variants::VariantType>,
        fields: &'a [SemanticField],
    ) -> Self {
        Self {
            types,
            fields,
            memo: HashMap::new(),
            remaining: MAX_COVERAGE_STEPS,
        }
    }

    fn domain(&self, ty: &ValueType) -> Domain {
        if !ty.is_pointer() {
            if let Some(owner) = ty.as_aggregate_identity().and_then(|id| id.symbol) {
                if self.types.contains_key(&owner) {
                    return Domain::Variant(owner);
                }
            }
        }
        Domain::Open
    }

    pub(super) fn useful(
        &mut self,
        previous: &[Pattern],
        pattern: &Pattern,
        ty: &ValueType,
    ) -> Result<Option<Pattern>, &'static str> {
        let result = self.search(
            Query {
                matrix: previous.iter().cloned().map(|p| vec![p]).collect(),
                row: vec![pattern.clone()],
                domains: vec![self.domain(ty)],
            },
            0,
        )?;
        Ok(result.map(|mut row| row.remove(0)))
    }

    fn search(&mut self, query: Query, depth: usize) -> Result<Option<Vec<Pattern>>, &'static str> {
        // Count both calls and copied matrix cells, so memoization cannot hide
        // pathological width/product growth. Exhaustion is never coverage.
        let work = 1 + query.row.len() + query.matrix.iter().map(Vec::len).sum::<usize>();
        if depth > 128 || work > self.remaining {
            return Err(
                "pattern coverage analysis exceeds its bounded work/depth limit; simplify the CASE patterns",
            );
        }
        self.remaining -= work;
        if let Some(result) = self.memo.get(&query) {
            return Ok(result.clone());
        }
        let result = if query.row.is_empty() {
            query.matrix.is_empty().then(Vec::new)
        } else {
            self.column(&query, depth)?
        };
        self.memo.insert(query, result.clone());
        Ok(result)
    }

    fn column(&mut self, q: &Query, depth: usize) -> Result<Option<Vec<Pattern>>, &'static str> {
        match &q.row[0] {
            Pattern::Constructor(id, fields) => self.constructor(q, *id, fields, depth),
            Pattern::Literal(bits) => {
                let matrix = q
                    .matrix
                    .iter()
                    .filter(|row| {
                        matches!(row[0], Pattern::Wild) || row[0] == Pattern::Literal(*bits)
                    })
                    .map(|row| row[1..].to_vec())
                    .collect();
                Ok(self
                    .search(
                        Query {
                            matrix,
                            row: q.row[1..].to_vec(),
                            domains: q.domains[1..].to_vec(),
                        },
                        depth + 1,
                    )?
                    .map(|mut row| {
                        row.insert(0, Pattern::Literal(*bits));
                        row
                    }))
            }
            Pattern::Wild => {
                // A column containing only wildcards does not need a product
                // expansion, regardless of its finite constructor domain.
                if let Domain::Variant(owner) = q.domains[0] {
                    if q.matrix.iter().any(|row| !matches!(row[0], Pattern::Wild))
                        || q.matrix.is_empty()
                    {
                        for constructor in self.types[&owner].constructors.clone() {
                            let fields = vec![Pattern::Wild; constructor.fields.len()];
                            if let Some(witness) =
                                self.constructor(q, constructor.id, &fields, depth)?
                            {
                                return Ok(Some(witness));
                            }
                        }
                        return Ok(None);
                    }
                }
                let matrix = q
                    .matrix
                    .iter()
                    .filter(|row| matches!(row[0], Pattern::Wild))
                    .map(|row| row[1..].to_vec())
                    .collect();
                Ok(self
                    .search(
                        Query {
                            matrix,
                            row: q.row[1..].to_vec(),
                            domains: q.domains[1..].to_vec(),
                        },
                        depth + 1,
                    )?
                    .map(|mut row| {
                        row.insert(0, Pattern::Wild);
                        row
                    }))
            }
        }
    }

    fn constructor(
        &mut self,
        q: &Query,
        id: VariantConstructorId,
        fields: &[Pattern],
        depth: usize,
    ) -> Result<Option<Vec<Pattern>>, &'static str> {
        let constructor = &self.types[&id.owner].constructors[usize::from(id.tag) - 1];
        let width = fields.len();
        let domains = constructor
            .fields
            .iter()
            .map(|id| self.domain(&self.fields[id.0].ty))
            .chain(q.domains[1..].iter().copied())
            .collect();
        let matrix = q
            .matrix
            .iter()
            .filter_map(|row| {
                let mut head = match &row[0] {
                    Pattern::Wild => vec![Pattern::Wild; width],
                    Pattern::Constructor(other, fields) if *other == id => fields.clone(),
                    _ => return None,
                };
                head.extend_from_slice(&row[1..]);
                Some(head)
            })
            .collect();
        let mut row = fields.to_vec();
        row.extend_from_slice(&q.row[1..]);
        Ok(self
            .search(
                Query {
                    matrix,
                    row,
                    domains,
                },
                depth + 1,
            )?
            .map(|mut witness| {
                let tail = witness.split_off(width);
                let mut result = vec![Pattern::Constructor(id, witness)];
                result.extend(tail);
                result
            }))
    }

    pub(super) fn display(&self, pattern: &Pattern) -> String {
        match pattern {
            Pattern::Wild => "_".into(),
            Pattern::Literal(bits) => format!("${bits:X}"),
            Pattern::Constructor(id, fields) => {
                let ty = &self.types[&id.owner];
                let constructor = &ty.constructors[usize::from(id.tag) - 1];
                let name = format!("{}.{}", ty.identity.name, constructor.name);
                if fields.is_empty() {
                    name
                } else {
                    format!(
                        "{name}({})",
                        fields
                            .iter()
                            .map(|p| self.display(p))
                            .collect::<Vec<_>>()
                            .join(",")
                    )
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_usefulness_matches_an_independent_finite_set_oracle() {
        let ast = crate::parser::parse(
            &crate::lexer::tokenize(
                "TYPE Flag=VARIANT [OFF ON] TYPE Pair=VARIANT [BOTH [Flag x,y]] PROC Main() RETURN",
            )
            .unwrap(),
        )
        .unwrap();
        let model = super::super::analyze_with_options(&ast, SemanticOptions::modern()).unwrap();
        let flag = model
            .variants
            .types
            .values()
            .find(|ty| ty.identity.name == "Flag")
            .unwrap();
        let pair = model
            .variants
            .types
            .values()
            .find(|ty| ty.identity.name == "Pair")
            .unwrap();
        let heads = [
            Pattern::Wild,
            Pattern::Constructor(flag.constructors[0].id, Vec::new()),
            Pattern::Constructor(flag.constructors[1].id, Vec::new()),
        ];
        let mut candidates = Vec::new();
        for x in 0..3 {
            for y in 0..3 {
                let mask = (0..4)
                    .filter(|value| {
                        (x == 0 || x - 1 == value / 2) && (y == 0 || y - 1 == value % 2)
                    })
                    .fold(0u8, |mask, value| mask | (1 << value));
                candidates.push((
                    Pattern::Constructor(
                        pair.constructors[0].id,
                        vec![heads[x].clone(), heads[y].clone()],
                    ),
                    mask,
                ));
            }
        }
        let ty = ValueType::aggregate(pair.identity.clone());
        for subset in 0..512 {
            let previous: Vec<_> = candidates
                .iter()
                .enumerate()
                .filter(|(i, _)| subset & (1 << i) != 0)
                .map(|(_, (p, _))| p.clone())
                .collect();
            let covered = candidates
                .iter()
                .enumerate()
                .filter(|(i, _)| subset & (1 << i) != 0)
                .fold(0u8, |mask, (_, (_, values))| mask | values);
            let mut coverage = Coverage::new(&model.variants.types, &model.fields);
            for (pattern, values) in &candidates {
                assert_eq!(
                    coverage.useful(&previous, pattern, &ty).unwrap().is_some(),
                    values & !covered != 0,
                    "subset {subset}, pattern {pattern:?}"
                );
            }
            assert_eq!(
                coverage
                    .useful(&previous, &Pattern::Wild, &ty)
                    .unwrap()
                    .is_some(),
                covered != 15,
                "subset {subset}"
            );
        }
    }
}
