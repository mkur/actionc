use crate::ast::{FundType, RoutineKind};
use crate::lexer::NumberKind;

use super::{FieldId, ValueType, ValueTypeBase};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScalarType {
    Byte,
    Card,
    Char,
    Int,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScalarSignedness {
    Unsigned,
    Signed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PointerType {
    pub pointee: Box<ValueType>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArrayType {
    pub element: Box<ValueType>,
    pub length: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordType {
    pub name: String,
    pub fields: Vec<RecordFieldType>,
    pub size: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordIdentity {
    pub name: String,
    pub is_pointer: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordIdentityRef<'a> {
    pub name: &'a str,
    pub is_pointer: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordFieldType {
    pub id: Option<FieldId>,
    pub name: String,
    pub ty: ValueType,
    pub offset: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallableType {
    pub kind: RoutineKind,
    pub params: Vec<ValueType>,
    pub variadic: Option<ValueType>,
    pub return_type: Option<ValueType>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueTypeKind {
    Scalar(ScalarType),
    Pointer(PointerType),
    CallablePointer(CallableType),
    Record(String),
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeCompatibility {
    Exact,
    ScalarAssignment,
    ScalarArgument,
    PointerExact,
    PointerAddress,
    AddressAsCard,
    Error,
    Incompatible,
}

impl TypeCompatibility {
    pub fn is_allowed(self) -> bool {
        !matches!(self, Self::Incompatible)
    }
}

impl CallableType {
    pub fn new(
        kind: RoutineKind,
        params: impl IntoIterator<Item = ValueType>,
        return_type: Option<ValueType>,
    ) -> Self {
        Self {
            kind,
            params: params.into_iter().collect(),
            variadic: None,
            return_type,
        }
    }

    pub fn new_variadic(
        kind: RoutineKind,
        params: impl IntoIterator<Item = ValueType>,
        variadic: ValueType,
        return_type: Option<ValueType>,
    ) -> Self {
        Self {
            kind,
            params: params.into_iter().collect(),
            variadic: Some(variadic),
            return_type,
        }
    }

    pub fn from_routine_kind(
        kind: RoutineKind,
        params: impl IntoIterator<Item = ValueType>,
    ) -> Self {
        let return_type = match kind {
            RoutineKind::Proc => None,
            RoutineKind::Func { return_type } => Some(ValueType::fund(return_type)),
        };
        Self::new(kind, params, return_type)
    }

    pub fn unknown_proc() -> Self {
        Self::new(RoutineKind::Proc, Vec::new(), None)
    }

    pub fn from_return_fund(return_type: FundType) -> Self {
        Self::new(
            RoutineKind::Func { return_type },
            Vec::new(),
            Some(ValueType::fund(return_type)),
        )
    }

    pub fn is_function(&self) -> bool {
        self.return_type.is_some()
    }

    pub fn is_proc(&self) -> bool {
        self.return_type.is_none()
    }
}

impl ArrayType {
    pub fn new(element: ValueType, length: Option<u16>) -> Self {
        Self {
            element: Box::new(element),
            length,
        }
    }

    pub fn pointer_type(&self) -> ValueType {
        ValueType::pointer_to((*self.element).clone())
    }

    pub fn element_width_bytes(&self) -> Option<u16> {
        self.element.value_width_bytes()
    }

    pub fn total_width_bytes(&self) -> Option<u16> {
        self.length
            .zip(self.element_width_bytes())
            .map(|(length, width)| length.saturating_mul(width))
    }
}

impl RecordType {
    pub fn new(
        name: impl Into<String>,
        fields: impl IntoIterator<Item = RecordFieldType>,
        size: u16,
    ) -> Self {
        Self {
            name: name.into(),
            fields: fields.into_iter().collect(),
            size,
        }
    }

    pub fn value_type(&self) -> ValueType {
        ValueType::record(self.name.clone())
    }

    pub fn pointer_type(&self) -> ValueType {
        ValueType::pointer_to(self.value_type())
    }

    pub fn field(&self, name: &str) -> Option<&RecordFieldType> {
        self.fields
            .iter()
            .find(|field| field.name.eq_ignore_ascii_case(name))
    }
}

impl RecordIdentityRef<'_> {
    pub fn to_owned(self) -> RecordIdentity {
        RecordIdentity {
            name: self.name.to_string(),
            is_pointer: self.is_pointer,
        }
    }
}

impl ScalarType {
    pub fn from_fund(fund: FundType) -> Self {
        match fund {
            FundType::Byte => Self::Byte,
            FundType::Card => Self::Card,
            FundType::Char => Self::Char,
            FundType::Int => Self::Int,
        }
    }

    pub fn from_number_kind(kind: NumberKind) -> Option<Self> {
        match kind {
            NumberKind::Byte => Some(Self::Byte),
            NumberKind::Card => Some(Self::Card),
            NumberKind::Int => Some(Self::Int),
            NumberKind::Real => None,
        }
    }

    pub fn fund_type(self) -> FundType {
        match self {
            Self::Byte => FundType::Byte,
            Self::Card => FundType::Card,
            Self::Char => FundType::Char,
            Self::Int => FundType::Int,
        }
    }

    pub fn width_bytes(self) -> u16 {
        match self {
            Self::Byte | Self::Char => 1,
            Self::Card | Self::Int => 2,
        }
    }

    pub fn signedness(self) -> ScalarSignedness {
        match self {
            Self::Int => ScalarSignedness::Signed,
            Self::Byte | Self::Card | Self::Char => ScalarSignedness::Unsigned,
        }
    }

    pub fn is_signed(self) -> bool {
        self.signedness() == ScalarSignedness::Signed
    }

    pub fn can_assign_from(self, actual: Self) -> bool {
        self == actual
            || matches!(
                (self, actual),
                (Self::Byte, Self::Char)
                    | (Self::Char, Self::Byte)
                    | (Self::Int, Self::Byte | Self::Char)
                    | (Self::Card, Self::Byte | Self::Char | Self::Int)
            )
    }

    pub fn promote_binary(left: Self, right: Self) -> Self {
        match (left, right) {
            (Self::Card, _) | (_, Self::Card) => Self::Card,
            (Self::Int, _) | (_, Self::Int) => Self::Int,
            (Self::Byte, Self::Byte) => Self::Byte,
            (Self::Char, Self::Char) => Self::Char,
            (Self::Byte | Self::Char, Self::Byte | Self::Char) => Self::Byte,
        }
    }
}

impl ValueType {
    pub fn scalar(scalar: ScalarType) -> Self {
        Self {
            base: ValueTypeBase::Fund(scalar.fund_type()),
            pointer: false,
        }
    }

    pub fn fund(fund: FundType) -> Self {
        Self::scalar(ScalarType::from_fund(fund))
    }

    pub fn record(name: impl Into<String>) -> Self {
        Self {
            base: ValueTypeBase::Named(name.into()),
            pointer: false,
        }
    }

    pub fn record_pointer(name: impl Into<String>) -> Self {
        Self::pointer_to(Self::record(name))
    }

    pub fn pointer_to(mut pointee: ValueType) -> Self {
        pointee.pointer = true;
        pointee
    }

    pub fn callable_pointer(callable: CallableType) -> Self {
        Self {
            base: ValueTypeBase::Callable(Box::new(callable)),
            pointer: false,
        }
    }

    pub fn as_callable_pointer(&self) -> Option<&CallableType> {
        match (&self.base, self.pointer) {
            (ValueTypeBase::Callable(callable), false) => Some(callable),
            _ => None,
        }
    }

    pub fn as_pointer(&self) -> Option<PointerType> {
        if !self.pointer {
            return None;
        }

        Some(PointerType {
            pointee: Box::new(self.pointee_type()),
        })
    }

    pub fn pointee_type(&self) -> Self {
        let mut pointee = self.clone();
        pointee.pointer = false;
        pointee
    }

    pub fn is_pointer(&self) -> bool {
        self.pointer
    }

    pub fn as_scalar(&self) -> Option<ScalarType> {
        if self.pointer {
            return None;
        }
        match self.base {
            ValueTypeBase::Fund(fund) => Some(ScalarType::from_fund(fund)),
            ValueTypeBase::Named(_) | ValueTypeBase::Callable(_) | ValueTypeBase::Error => None,
        }
    }

    pub fn scalar_width_bytes(&self) -> Option<u16> {
        self.as_scalar().map(ScalarType::width_bytes)
    }

    pub fn is_numeric_scalar(&self) -> bool {
        self.as_scalar().is_some()
    }

    pub fn value_width_bytes(&self) -> Option<u16> {
        match self.kind() {
            ValueTypeKind::Scalar(scalar) => Some(scalar.width_bytes()),
            ValueTypeKind::Pointer(_) => Some(2),
            ValueTypeKind::CallablePointer(_) => Some(2),
            ValueTypeKind::Record(_) | ValueTypeKind::Error => None,
        }
    }

    pub fn is_byte_sized_value(&self) -> bool {
        self.value_width_bytes() == Some(1)
    }

    pub fn is_word_sized_value(&self) -> bool {
        self.value_width_bytes() == Some(2)
    }

    pub fn kind(&self) -> ValueTypeKind {
        if let Some(pointer) = self.as_pointer() {
            return ValueTypeKind::Pointer(pointer);
        }

        match &self.base {
            ValueTypeBase::Fund(fund) => ValueTypeKind::Scalar(ScalarType::from_fund(*fund)),
            ValueTypeBase::Named(name) => ValueTypeKind::Record(name.clone()),
            ValueTypeBase::Callable(callable) if !self.pointer => {
                ValueTypeKind::CallablePointer((**callable).clone())
            }
            ValueTypeBase::Callable(_) => ValueTypeKind::Error,
            ValueTypeBase::Error => ValueTypeKind::Error,
        }
    }

    pub fn as_record_name(&self) -> Option<&str> {
        self.as_record_identity()
            .filter(|identity| !identity.is_pointer)
            .map(|identity| identity.name)
    }

    pub fn is_record(&self) -> bool {
        self.as_record_name().is_some()
    }

    pub fn as_record_base_name(&self) -> Option<&str> {
        self.as_record_identity().map(|identity| identity.name)
    }

    pub fn has_record_base(&self) -> bool {
        self.as_record_base_name().is_some()
    }

    pub fn as_record_identity(&self) -> Option<RecordIdentityRef<'_>> {
        match &self.base {
            ValueTypeBase::Named(name) => Some(RecordIdentityRef {
                name,
                is_pointer: self.pointer,
            }),
            ValueTypeBase::Fund(_) | ValueTypeBase::Callable(_) | ValueTypeBase::Error => None,
        }
    }

    pub fn record_identity(&self) -> Option<RecordIdentity> {
        self.as_record_identity().map(RecordIdentityRef::to_owned)
    }

    pub fn is_record_pointer(&self) -> bool {
        self.as_record_identity()
            .is_some_and(|identity| identity.is_pointer)
    }

    pub fn same_record_family(&self, other: &Self) -> bool {
        self.as_record_base_name()
            .zip(other.as_record_base_name())
            .is_some_and(|(left, right)| left.eq_ignore_ascii_case(right))
    }

    pub fn assignment_compatibility(&self, actual: &Self) -> TypeCompatibility {
        if self.is_error() || actual.is_error() {
            return TypeCompatibility::Error;
        }
        if let ValueTypeKind::Pointer(expected) = self.kind() {
            return pointer_compatibility(&expected, actual);
        }
        if self == actual {
            return TypeCompatibility::Exact;
        }
        if matches!(self.kind(), ValueTypeKind::CallablePointer(_))
            && matches!(actual.kind(), ValueTypeKind::Scalar(ScalarType::Card))
        {
            return TypeCompatibility::PointerAddress;
        }
        if matches!(self.kind(), ValueTypeKind::Scalar(ScalarType::Card))
            && matches!(
                actual.kind(),
                ValueTypeKind::Pointer(_) | ValueTypeKind::CallablePointer(_)
            )
        {
            return TypeCompatibility::AddressAsCard;
        }
        if actual.is_pointer() || matches!(actual.kind(), ValueTypeKind::CallablePointer(_)) {
            return TypeCompatibility::Incompatible;
        }
        match (self.as_scalar(), actual.as_scalar()) {
            (Some(expected), Some(actual)) if expected.can_assign_from(actual) => {
                TypeCompatibility::ScalarAssignment
            }
            _ => TypeCompatibility::Incompatible,
        }
    }

    pub fn argument_compatibility(&self, actual: &Self) -> TypeCompatibility {
        if self.is_error() || actual.is_error() {
            return TypeCompatibility::Error;
        }
        if let ValueTypeKind::Pointer(expected) = self.kind() {
            return pointer_compatibility(&expected, actual);
        }
        if let (ValueTypeKind::Scalar(_), ValueTypeKind::Scalar(_)) = (self.kind(), actual.kind()) {
            return if self == actual {
                TypeCompatibility::Exact
            } else {
                TypeCompatibility::ScalarArgument
            };
        }

        self.assignment_compatibility(actual)
    }
}

fn pointer_compatibility(expected: &PointerType, actual: &ValueType) -> TypeCompatibility {
    match actual.kind() {
        ValueTypeKind::Pointer(actual)
            if pointer_pointees_compatible(&expected.pointee, &actual.pointee) =>
        {
            TypeCompatibility::PointerExact
        }
        ValueTypeKind::Scalar(ScalarType::Card) => TypeCompatibility::PointerAddress,
        ValueTypeKind::Error => TypeCompatibility::Error,
        _ => TypeCompatibility::Incompatible,
    }
}

fn pointer_pointees_compatible(expected: &ValueType, actual: &ValueType) -> bool {
    expected == actual
        || matches!(
            (
                &expected.base,
                expected.pointer,
                &actual.base,
                actual.pointer
            ),
            (
                ValueTypeBase::Fund(FundType::Byte),
                false,
                ValueTypeBase::Fund(FundType::Char),
                false
            ) | (
                ValueTypeBase::Fund(FundType::Char),
                false,
                ValueTypeBase::Fund(FundType::Byte),
                false
            )
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_width_and_signedness_are_canonical() {
        assert_eq!(ScalarType::Byte.width_bytes(), 1);
        assert_eq!(ScalarType::Char.width_bytes(), 1);
        assert_eq!(ScalarType::Card.width_bytes(), 2);
        assert_eq!(ScalarType::Int.width_bytes(), 2);

        assert!(!ScalarType::Byte.is_signed());
        assert!(!ScalarType::Card.is_signed());
        assert!(!ScalarType::Char.is_signed());
        assert!(ScalarType::Int.is_signed());
    }

    #[test]
    fn scalar_assignment_matches_current_action_rules() {
        assert!(ScalarType::Byte.can_assign_from(ScalarType::Char));
        assert!(ScalarType::Char.can_assign_from(ScalarType::Byte));
        assert!(ScalarType::Int.can_assign_from(ScalarType::Byte));
        assert!(ScalarType::Card.can_assign_from(ScalarType::Int));

        assert!(!ScalarType::Byte.can_assign_from(ScalarType::Card));
        assert!(!ScalarType::Int.can_assign_from(ScalarType::Card));
    }

    #[test]
    fn scalar_promotion_preserves_existing_precedence() {
        assert_eq!(
            ScalarType::promote_binary(ScalarType::Byte, ScalarType::Char),
            ScalarType::Byte
        );
        assert_eq!(
            ScalarType::promote_binary(ScalarType::Char, ScalarType::Char),
            ScalarType::Char
        );
        assert_eq!(
            ScalarType::promote_binary(ScalarType::Int, ScalarType::Byte),
            ScalarType::Int
        );
        assert_eq!(
            ScalarType::promote_binary(ScalarType::Int, ScalarType::Card),
            ScalarType::Card
        );
    }

    #[test]
    fn pointer_type_preserves_pointee_type() {
        let byte_pointer = ValueType::pointer_to(ValueType::fund(FundType::Byte));
        let pointer = byte_pointer.as_pointer().expect("pointer type");

        assert!(byte_pointer.is_pointer());
        assert_eq!(*pointer.pointee, ValueType::fund(FundType::Byte));
        assert_eq!(byte_pointer.pointee_type(), ValueType::fund(FundType::Byte));
        assert_eq!(byte_pointer.as_scalar(), None);
    }

    #[test]
    fn pointer_type_preserves_named_pointee() {
        let record_pointer = ValueType::record_pointer("Pair");
        let pointer = record_pointer.as_pointer().expect("pointer type");

        assert_eq!(*pointer.pointee, ValueType::record("Pair"));
    }

    #[test]
    fn value_type_kind_classifies_scalar_pointer_record_and_error() {
        assert_eq!(
            ValueType::fund(FundType::Byte).kind(),
            ValueTypeKind::Scalar(ScalarType::Byte)
        );

        let byte_pointer = ValueType::pointer_to(ValueType::fund(FundType::Byte));
        assert_eq!(
            byte_pointer.kind(),
            ValueTypeKind::Pointer(PointerType {
                pointee: Box::new(ValueType::fund(FundType::Byte))
            })
        );

        let record = ValueType::record("Pair");
        assert_eq!(record.kind(), ValueTypeKind::Record("Pair".to_string()));
        assert_eq!(record.as_record_name(), Some("Pair"));
        assert_eq!(record.as_record_base_name(), Some("Pair"));
        assert!(record.is_record());
        assert!(record.has_record_base());

        let record_pointer = ValueType::record_pointer("Pair");
        assert_eq!(record_pointer.as_record_name(), None);
        assert_eq!(record_pointer.as_record_base_name(), Some("Pair"));
        assert!(record_pointer.has_record_base());
        assert!(record_pointer.is_record_pointer());
        assert!(record.same_record_family(&record_pointer));
        assert_eq!(
            record_pointer.record_identity(),
            Some(RecordIdentity {
                name: "Pair".to_string(),
                is_pointer: true
            })
        );

        assert_eq!(ValueType::error().kind(), ValueTypeKind::Error);
    }

    #[test]
    fn value_type_width_includes_scalars_and_pointers() {
        assert_eq!(ValueType::fund(FundType::Byte).value_width_bytes(), Some(1));
        assert_eq!(ValueType::fund(FundType::Char).value_width_bytes(), Some(1));
        assert_eq!(ValueType::fund(FundType::Card).value_width_bytes(), Some(2));
        assert_eq!(ValueType::fund(FundType::Int).value_width_bytes(), Some(2));

        let pointer = ValueType::pointer_to(ValueType::fund(FundType::Byte));
        assert_eq!(pointer.value_width_bytes(), Some(2));
        assert!(pointer.is_word_sized_value());
        assert!(!pointer.is_byte_sized_value());

        let record = ValueType::record("Pair");
        assert_eq!(record.value_width_bytes(), None);
        assert_eq!(ValueType::error().value_width_bytes(), None);
    }

    #[test]
    fn type_compatibility_classifies_assignment_and_argument_rules() {
        let byte = ValueType::fund(FundType::Byte);
        let card = ValueType::fund(FundType::Card);
        let int = ValueType::fund(FundType::Int);
        let byte_pointer = ValueType::pointer_to(byte.clone());
        let card_pointer = ValueType::pointer_to(card.clone());

        assert_eq!(
            byte.assignment_compatibility(&byte),
            TypeCompatibility::Exact
        );
        assert_eq!(
            card.assignment_compatibility(&int),
            TypeCompatibility::ScalarAssignment
        );
        assert_eq!(
            byte.assignment_compatibility(&card),
            TypeCompatibility::Incompatible
        );
        assert_eq!(
            byte.argument_compatibility(&card),
            TypeCompatibility::ScalarArgument
        );
        assert_eq!(
            byte_pointer.assignment_compatibility(&byte_pointer),
            TypeCompatibility::PointerExact
        );
        assert_eq!(
            byte_pointer.assignment_compatibility(&card),
            TypeCompatibility::PointerAddress
        );
        assert_eq!(
            byte_pointer.assignment_compatibility(&card_pointer),
            TypeCompatibility::Incompatible
        );
        assert_eq!(
            card.assignment_compatibility(&byte_pointer),
            TypeCompatibility::AddressAsCard
        );
    }

    #[test]
    fn callable_type_models_proc_and_function_signatures() {
        let proc = CallableType::unknown_proc();
        assert_eq!(proc.kind, RoutineKind::Proc);
        assert!(proc.params.is_empty());
        assert_eq!(proc.return_type, None);
        assert!(proc.is_proc());

        let function = CallableType::from_routine_kind(
            RoutineKind::Func {
                return_type: FundType::Byte,
            },
            [ValueType::fund(FundType::Card)],
        );
        assert_eq!(
            function.kind,
            RoutineKind::Func {
                return_type: FundType::Byte
            }
        );
        assert_eq!(function.params, vec![ValueType::fund(FundType::Card)]);
        assert_eq!(function.return_type, Some(ValueType::fund(FundType::Byte)));
        assert!(function.is_function());
    }

    #[test]
    fn array_type_models_element_pointer_and_optional_size() {
        let bytes = ArrayType::new(ValueType::fund(FundType::Byte), Some(10));
        assert_eq!(*bytes.element, ValueType::fund(FundType::Byte));
        assert_eq!(bytes.length, Some(10));
        assert_eq!(
            bytes.pointer_type(),
            ValueType::pointer_to(ValueType::fund(FundType::Byte))
        );
        assert_eq!(bytes.element_width_bytes(), Some(1));
        assert_eq!(bytes.total_width_bytes(), Some(10));

        let cards = ArrayType::new(ValueType::fund(FundType::Card), None);
        assert_eq!(
            cards.pointer_type(),
            ValueType::pointer_to(ValueType::fund(FundType::Card))
        );
        assert_eq!(cards.element_width_bytes(), Some(2));
        assert_eq!(cards.total_width_bytes(), None);
    }

    #[test]
    fn record_type_models_fields_size_and_pointer_type() {
        let record = RecordType::new(
            "Pair",
            [
                RecordFieldType {
                    id: Some(FieldId(1)),
                    name: "tag".to_string(),
                    ty: ValueType::fund(FundType::Byte),
                    offset: 0,
                },
                RecordFieldType {
                    id: Some(FieldId(2)),
                    name: "word".to_string(),
                    ty: ValueType::fund(FundType::Card),
                    offset: 1,
                },
            ],
            3,
        );

        assert_eq!(record.value_type(), ValueType::record("Pair"));
        assert_eq!(
            record.pointer_type(),
            ValueType::pointer_to(ValueType::record("Pair"))
        );
        assert_eq!(record.size, 3);
        assert_eq!(record.field("TAG").map(|field| field.offset), Some(0));
        assert_eq!(
            record.field("word").map(|field| field.ty.clone()),
            Some(ValueType::fund(FundType::Card))
        );
    }
}
