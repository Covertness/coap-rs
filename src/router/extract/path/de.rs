//! Deserialization logic for path parameters in the router extractors.

use crate::router::{extract::path::PathDeserializationError, util::PercentDecodedStr};
use serde::{
    de::{self, DeserializeSeed, EnumAccess, Error, MapAccess, SeqAccess, VariantAccess, Visitor},
    forward_to_deserialize_any, Deserializer,
};
use std::{any::type_name, sync::Arc};

macro_rules! unsupported_type {
    ($trait_fn:ident) => {
        fn $trait_fn<V>(self, _: V) -> Result<V::Value, Self::Error>
        where
            V: Visitor<'de>,
        {
            Err(PathDeserializationError::UnsupportedType {
                name: type_name::<V::Value>(),
            })
        }
    };
}

macro_rules! parse_single_value {
    ($trait_fn:ident, $visit_fn:ident, $ty:literal) => {
        fn $trait_fn<V>(self, visitor: V) -> Result<V::Value, Self::Error>
        where
            V: Visitor<'de>,
        {
            if self.url_params.len() != 1 {
                return Err(PathDeserializationError::WrongNumberOfParameters {
                    got: self.url_params.len(),
                    expected: 1,
                });
            }

            let value =
                self.url_params[0]
                    .1
                    .parse()
                    .map_err(|_| PathDeserializationError::ParseError {
                        value: self.url_params[0].1.as_str().to_owned(),
                        expected_type: $ty,
                    })?;
            visitor.$visit_fn(value)
        }
    };
}

pub(crate) struct PathDeserializer<'de> {
    url_params: &'de [(Arc<str>, PercentDecodedStr)],
}

impl<'de> PathDeserializer<'de> {
    #[inline]
    pub(crate) fn new(url_params: &'de [(Arc<str>, PercentDecodedStr)]) -> Self {
        PathDeserializer { url_params }
    }
}

impl<'de> Deserializer<'de> for PathDeserializer<'de> {
    type Error = PathDeserializationError;

    unsupported_type!(deserialize_bytes);
    unsupported_type!(deserialize_option);
    unsupported_type!(deserialize_identifier);
    unsupported_type!(deserialize_ignored_any);

    parse_single_value!(deserialize_bool, visit_bool, "bool");
    parse_single_value!(deserialize_i8, visit_i8, "i8");
    parse_single_value!(deserialize_i16, visit_i16, "i16");
    parse_single_value!(deserialize_i32, visit_i32, "i32");
    parse_single_value!(deserialize_i64, visit_i64, "i64");
    parse_single_value!(deserialize_i128, visit_i128, "i128");
    parse_single_value!(deserialize_u8, visit_u8, "u8");
    parse_single_value!(deserialize_u16, visit_u16, "u16");
    parse_single_value!(deserialize_u32, visit_u32, "u32");
    parse_single_value!(deserialize_u64, visit_u64, "u64");
    parse_single_value!(deserialize_u128, visit_u128, "u128");
    parse_single_value!(deserialize_f32, visit_f32, "f32");
    parse_single_value!(deserialize_f64, visit_f64, "f64");
    parse_single_value!(deserialize_string, visit_string, "String");
    parse_single_value!(deserialize_byte_buf, visit_string, "String");
    parse_single_value!(deserialize_char, visit_char, "char");

    fn deserialize_any<V>(self, v: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_str(v)
    }

    fn deserialize_str<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        if self.url_params.len() != 1 {
            return Err(PathDeserializationError::WrongNumberOfParameters {
                got: self.url_params.len(),
                expected: 1,
            });
        }
        let value = &self.url_params[0].1;
        visitor.visit_borrowed_str(value)
    }

    fn deserialize_unit<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_unit()
    }

    fn deserialize_unit_struct<V>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_unit()
    }

    fn deserialize_newtype_struct<V>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_seq<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_seq(SeqDeserializer {
            params: self.url_params,
            idx: 0,
        })
    }

    fn deserialize_tuple<V>(self, len: usize, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        if self.url_params.len() != len {
            return Err(PathDeserializationError::WrongNumberOfParameters {
                got: self.url_params.len(),
                expected: len,
            });
        }
        visitor.visit_seq(SeqDeserializer {
            params: self.url_params,
            idx: 0,
        })
    }

    fn deserialize_tuple_struct<V>(
        self,
        _name: &'static str,
        len: usize,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        if self.url_params.len() != len {
            return Err(PathDeserializationError::WrongNumberOfParameters {
                got: self.url_params.len(),
                expected: len,
            });
        }
        visitor.visit_seq(SeqDeserializer {
            params: self.url_params,
            idx: 0,
        })
    }

    fn deserialize_map<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_map(MapDeserializer {
            params: self.url_params,
            value: None,
            key: None,
        })
    }

    fn deserialize_struct<V>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_map(visitor)
    }

    fn deserialize_enum<V>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        if self.url_params.len() != 1 {
            return Err(PathDeserializationError::WrongNumberOfParameters {
                got: self.url_params.len(),
                expected: 1,
            });
        }

        visitor.visit_enum(EnumDeserializer {
            value: &self.url_params[0].1,
        })
    }
}

struct MapDeserializer<'de> {
    params: &'de [(Arc<str>, PercentDecodedStr)],
    key: Option<KeyOrIdx<'de>>,
    value: Option<&'de PercentDecodedStr>,
}

impl<'de> MapAccess<'de> for MapDeserializer<'de> {
    type Error = PathDeserializationError;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, Self::Error>
    where
        K: DeserializeSeed<'de>,
    {
        match self.params.split_first() {
            Some(((key, value), tail)) => {
                self.value = Some(value);
                self.params = tail;
                self.key = Some(KeyOrIdx::Key(key));
                seed.deserialize(KeyDeserializer { key }).map(Some)
            }
            None => Ok(None),
        }
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, Self::Error>
    where
        V: DeserializeSeed<'de>,
    {
        match self.value.take() {
            Some(value) => seed.deserialize(ValueDeserializer {
                key: self.key.take(),
                value,
            }),
            None => Err(PathDeserializationError::custom("value is missing")),
        }
    }
}

struct KeyDeserializer<'de> {
    key: &'de str,
}

macro_rules! parse_key {
    ($trait_fn:ident) => {
        fn $trait_fn<V>(self, visitor: V) -> Result<V::Value, Self::Error>
        where
            V: Visitor<'de>,
        {
            visitor.visit_str(&self.key)
        }
    };
}

impl<'de> Deserializer<'de> for KeyDeserializer<'de> {
    type Error = PathDeserializationError;

    parse_key!(deserialize_identifier);
    parse_key!(deserialize_str);
    parse_key!(deserialize_string);

    fn deserialize_any<V>(self, _visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        Err(PathDeserializationError::custom("Unexpected key type"))
    }

    forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char bytes
        byte_buf option unit unit_struct seq tuple
        tuple_struct map newtype_struct struct enum ignored_any
    }
}

macro_rules! parse_value {
    ($trait_fn:ident, $visit_fn:ident, $ty:literal) => {
        fn $trait_fn<V>(mut self, visitor: V) -> Result<V::Value, Self::Error>
        where
            V: Visitor<'de>,
        {
            let v = self.value.parse().map_err(|_| {
                if let Some(key) = self.key.take() {
                    match key {
                        KeyOrIdx::Key(key) => PathDeserializationError::ParseErrorAtKey {
                            key: key.to_owned(),
                            value: self.value.as_str().to_owned(),
                            expected_type: $ty,
                        },
                        KeyOrIdx::Idx { idx: index, key: _ } => {
                            PathDeserializationError::ParseErrorAtIndex {
                                index,
                                value: self.value.as_str().to_owned(),
                                expected_type: $ty,
                            }
                        }
                    }
                } else {
                    PathDeserializationError::ParseError {
                        value: self.value.as_str().to_owned(),
                        expected_type: $ty,
                    }
                }
            })?;
            visitor.$visit_fn(v)
        }
    };
}

#[derive(Debug)]
struct ValueDeserializer<'de> {
    key: Option<KeyOrIdx<'de>>,
    value: &'de PercentDecodedStr,
}

impl<'de> Deserializer<'de> for ValueDeserializer<'de> {
    type Error = PathDeserializationError;

    unsupported_type!(deserialize_map);
    unsupported_type!(deserialize_identifier);

    parse_value!(deserialize_bool, visit_bool, "bool");
    parse_value!(deserialize_i8, visit_i8, "i8");
    parse_value!(deserialize_i16, visit_i16, "i16");
    parse_value!(deserialize_i32, visit_i32, "i32");
    parse_value!(deserialize_i64, visit_i64, "i64");
    parse_value!(deserialize_i128, visit_i128, "i128");
    parse_value!(deserialize_u8, visit_u8, "u8");
    parse_value!(deserialize_u16, visit_u16, "u16");
    parse_value!(deserialize_u32, visit_u32, "u32");
    parse_value!(deserialize_u64, visit_u64, "u64");
    parse_value!(deserialize_u128, visit_u128, "u128");
    parse_value!(deserialize_f32, visit_f32, "f32");
    parse_value!(deserialize_f64, visit_f64, "f64");
    parse_value!(deserialize_string, visit_string, "String");
    parse_value!(deserialize_byte_buf, visit_string, "String");
    parse_value!(deserialize_char, visit_char, "char");

    fn deserialize_any<V>(self, v: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_str(v)
    }

    fn deserialize_str<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_borrowed_str(self.value)
    }

    fn deserialize_bytes<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_borrowed_bytes(self.value.as_bytes())
    }

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_some(self)
    }

    fn deserialize_unit<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_unit()
    }

    fn deserialize_unit_struct<V>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_unit()
    }

    fn deserialize_newtype_struct<V>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_tuple<V>(self, len: usize, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        struct PairDeserializer<'de> {
            key: Option<KeyOrIdx<'de>>,
            value: Option<&'de PercentDecodedStr>,
        }

        impl<'de> SeqAccess<'de> for PairDeserializer<'de> {
            type Error = PathDeserializationError;

            fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>, Self::Error>
            where
                T: DeserializeSeed<'de>,
            {
                match self.key.take() {
                    Some(KeyOrIdx::Idx { idx: _, key }) => {
                        return seed.deserialize(KeyDeserializer { key }).map(Some);
                    }
                    Some(KeyOrIdx::Key(_)) => {
                        return Err(PathDeserializationError::custom(
                            "array types are not supported",
                        ));
                    }
                    None => {}
                };

                self.value
                    .take()
                    .map(|value| seed.deserialize(ValueDeserializer { key: None, value }))
                    .transpose()
            }
        }

        if len == 2 {
            match self.key {
                Some(key) => visitor.visit_seq(PairDeserializer {
                    key: Some(key),
                    value: Some(self.value),
                }),
                // `self.key` is only `None` when deserializing maps so `deserialize_seq`
                // wouldn't be called for that
                None => unreachable!(),
            }
        } else {
            Err(PathDeserializationError::UnsupportedType {
                name: type_name::<V::Value>(),
            })
        }
    }

    fn deserialize_seq<V>(self, _visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        Err(PathDeserializationError::UnsupportedType {
            name: type_name::<V::Value>(),
        })
    }

    fn deserialize_tuple_struct<V>(
        self,
        _name: &'static str,
        _len: usize,
        _visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        Err(PathDeserializationError::UnsupportedType {
            name: type_name::<V::Value>(),
        })
    }

    fn deserialize_struct<V>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        _visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        Err(PathDeserializationError::UnsupportedType {
            name: type_name::<V::Value>(),
        })
    }

    fn deserialize_enum<V>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_enum(EnumDeserializer { value: self.value })
    }

    fn deserialize_ignored_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_unit()
    }
}

struct EnumDeserializer<'de> {
    value: &'de str,
}

impl<'de> EnumAccess<'de> for EnumDeserializer<'de> {
    type Error = PathDeserializationError;
    type Variant = UnitVariant;

    fn variant_seed<V>(self, seed: V) -> Result<(V::Value, Self::Variant), Self::Error>
    where
        V: de::DeserializeSeed<'de>,
    {
        Ok((
            seed.deserialize(KeyDeserializer { key: self.value })?,
            UnitVariant,
        ))
    }
}

struct UnitVariant;

impl<'de> VariantAccess<'de> for UnitVariant {
    type Error = PathDeserializationError;

    fn unit_variant(self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn newtype_variant_seed<T>(self, _seed: T) -> Result<T::Value, Self::Error>
    where
        T: DeserializeSeed<'de>,
    {
        Err(PathDeserializationError::UnsupportedType {
            name: "newtype enum variant",
        })
    }

    fn tuple_variant<V>(self, _len: usize, _visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        Err(PathDeserializationError::UnsupportedType {
            name: "tuple enum variant",
        })
    }

    fn struct_variant<V>(
        self,
        _fields: &'static [&'static str],
        _visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        Err(PathDeserializationError::UnsupportedType {
            name: "struct enum variant",
        })
    }
}

struct SeqDeserializer<'de> {
    params: &'de [(Arc<str>, PercentDecodedStr)],
    idx: usize,
}

impl<'de> SeqAccess<'de> for SeqDeserializer<'de> {
    type Error = PathDeserializationError;

    fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>, Self::Error>
    where
        T: DeserializeSeed<'de>,
    {
        match self.params.split_first() {
            Some(((key, value), tail)) => {
                self.params = tail;
                let idx = self.idx;
                self.idx += 1;
                Ok(Some(seed.deserialize(ValueDeserializer {
                    key: Some(KeyOrIdx::Idx { idx, key }),
                    value,
                })?))
            }
            None => Ok(None),
        }
    }
}

#[derive(Debug, Clone)]
enum KeyOrIdx<'de> {
    Key(&'de str),
    Idx { idx: usize, key: &'de str },
}

#[cfg(test)]
mod tests {
    use super::PathDeserializer;
    use crate::router::{extract::path::PathDeserializationError, util::PercentDecodedStr};
    use serde::{de::Visitor, Deserialize, Deserializer};
    use std::fmt;
    use std::{collections::HashMap, sync::Arc};

    fn params(input: &[(&str, &str)]) -> Vec<(Arc<str>, PercentDecodedStr)> {
        input
            .iter()
            .map(|(k, v)| {
                (
                    Arc::<str>::from(*k),
                    PercentDecodedStr::new(*v).expect("invalid encoded value in test"),
                )
            })
            .collect()
    }

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    struct UserPath {
        id: u32,
        name: String,
    }

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    enum UnitEnum {
        Active,
        Inactive,
    }

    #[allow(dead_code)]
    #[derive(Debug, Deserialize)]
    enum NewtypeEnum {
        Active(u32),
    }

    #[allow(dead_code)]
    #[derive(Debug, Deserialize)]
    enum TupleEnum {
        Active(u32, u32),
    }

    #[allow(dead_code)]
    #[derive(Debug, Deserialize)]
    enum StructEnum {
        Active { id: u32 },
    }

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    struct UnitStruct;

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    struct NewtypeStruct(String);

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    struct PairStruct(String, u32);

    #[derive(Debug, PartialEq, Eq)]
    struct AnyString(String);

    impl<'de> Deserialize<'de> for AnyString {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            struct AnyStringVisitor;

            impl<'de> Visitor<'de> for AnyStringVisitor {
                type Value = AnyString;

                fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                    formatter.write_str("a path value")
                }

                fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Self::Value, E>
                where
                    E: serde::de::Error,
                {
                    Ok(AnyString(value.to_string()))
                }

                fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
                where
                    E: serde::de::Error,
                {
                    Ok(AnyString(value.to_string()))
                }
            }

            deserializer.deserialize_any(AnyStringVisitor)
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    struct ForceBytes;

    impl<'de> Deserialize<'de> for ForceBytes {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            struct BytesVisitor;

            impl<'de> Visitor<'de> for BytesVisitor {
                type Value = ForceBytes;

                fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                    formatter.write_str("bytes")
                }

                fn visit_borrowed_bytes<E>(self, _value: &'de [u8]) -> Result<Self::Value, E>
                where
                    E: serde::de::Error,
                {
                    Ok(ForceBytes)
                }
            }

            deserializer.deserialize_bytes(BytesVisitor)
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    struct ForceIdentifier;

    impl<'de> Deserialize<'de> for ForceIdentifier {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            struct IdentifierVisitor;

            impl<'de> Visitor<'de> for IdentifierVisitor {
                type Value = ForceIdentifier;

                fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                    formatter.write_str("identifier")
                }

                fn visit_str<E>(self, _value: &str) -> Result<Self::Value, E>
                where
                    E: serde::de::Error,
                {
                    Ok(ForceIdentifier)
                }
            }

            deserializer.deserialize_identifier(IdentifierVisitor)
        }
    }

    #[test]
    fn deserialize_single_string_ok() {
        let p = params(&[("name", "alice")]);
        let value = String::deserialize(PathDeserializer::new(&p)).unwrap();
        assert_eq!(value, "alice");
    }

    #[test]
    fn deserialize_single_bool_wrong_number_of_params() {
        let p = params(&[("a", "true"), ("b", "false")]);
        let err = bool::deserialize(PathDeserializer::new(&p)).unwrap_err();

        assert_eq!(
            err,
            PathDeserializationError::WrongNumberOfParameters {
                got: 2,
                expected: 1,
            }
        );
    }

    #[test]
    fn deserialize_struct_ok() {
        let p = params(&[("id", "42"), ("name", "alice")]);
        let value = UserPath::deserialize(PathDeserializer::new(&p)).unwrap();

        assert_eq!(
            value,
            UserPath {
                id: 42,
                name: "alice".to_string(),
            }
        );
    }

    #[test]
    fn deserialize_struct_parse_error_at_key() {
        let p = params(&[("id", "abc"), ("name", "alice")]);
        let err = UserPath::deserialize(PathDeserializer::new(&p)).unwrap_err();

        assert_eq!(
            err,
            PathDeserializationError::ParseErrorAtKey {
                key: "id".to_string(),
                value: "abc".to_string(),
                expected_type: "u32",
            }
        );
    }

    #[test]
    fn deserialize_tuple_ok() {
        let p = params(&[("id", "7"), ("name", "bob")]);
        let value = <(u32, String)>::deserialize(PathDeserializer::new(&p)).unwrap();

        assert_eq!(value, (7, "bob".to_string()));
    }

    #[test]
    fn deserialize_tuple_parse_error_at_index() {
        let p = params(&[("id", "nope"), ("name", "bob")]);
        let err = <(u32, String)>::deserialize(PathDeserializer::new(&p)).unwrap_err();

        assert_eq!(
            err,
            PathDeserializationError::ParseErrorAtIndex {
                index: 0,
                value: "nope".to_string(),
                expected_type: "u32",
            }
        );
    }

    #[test]
    fn deserialize_tuple_wrong_number_of_params() {
        let p = params(&[("id", "7")]);
        let err = <(u32, String)>::deserialize(PathDeserializer::new(&p)).unwrap_err();

        assert_eq!(
            err,
            PathDeserializationError::WrongNumberOfParameters {
                got: 1,
                expected: 2,
            }
        );
    }

    #[test]
    fn deserialize_unit_enum_ok() {
        let p = params(&[("status", "Active")]);
        let value = UnitEnum::deserialize(PathDeserializer::new(&p)).unwrap();

        assert_eq!(value, UnitEnum::Active);
    }

    #[test]
    fn deserialize_enum_wrong_number_of_params() {
        let p = params(&[("status", "Active"), ("extra", "x")]);
        let err = UnitEnum::deserialize(PathDeserializer::new(&p)).unwrap_err();

        assert_eq!(
            err,
            PathDeserializationError::WrongNumberOfParameters {
                got: 2,
                expected: 1,
            }
        );
    }

    #[test]
    fn deserialize_enum_newtype_variant_is_unsupported() {
        let p = params(&[("status", "Active")]);
        let err = NewtypeEnum::deserialize(PathDeserializer::new(&p)).unwrap_err();

        assert_eq!(
            err,
            PathDeserializationError::UnsupportedType {
                name: "newtype enum variant",
            }
        );
    }

    #[test]
    fn deserialize_enum_tuple_variant_is_unsupported() {
        let p = params(&[("status", "Active")]);
        let err = TupleEnum::deserialize(PathDeserializer::new(&p)).unwrap_err();

        assert_eq!(
            err,
            PathDeserializationError::UnsupportedType {
                name: "tuple enum variant",
            }
        );
    }

    #[test]
    fn deserialize_enum_struct_variant_is_unsupported() {
        let p = params(&[("status", "Active")]);
        let err = StructEnum::deserialize(PathDeserializer::new(&p)).unwrap_err();

        assert_eq!(
            err,
            PathDeserializationError::UnsupportedType {
                name: "struct enum variant",
            }
        );
    }

    #[test]
    fn deserialize_map_with_non_string_key_is_rejected() {
        let p = params(&[("1", "alice")]);
        let err = HashMap::<u32, String>::deserialize(PathDeserializer::new(&p)).unwrap_err();

        assert_eq!(
            err,
            PathDeserializationError::Message("Unexpected key type".to_string())
        );
    }

    #[test]
    fn deserialize_option_is_unsupported() {
        let p = params(&[("name", "alice")]);
        let err = Option::<String>::deserialize(PathDeserializer::new(&p)).unwrap_err();

        assert_eq!(
            err,
            PathDeserializationError::UnsupportedType {
                name: "core::option::Option<alloc::string::String>",
            }
        );
    }

    #[test]
    fn deserialize_primitives_and_char_succeed() {
        let p = params(&[("value", "-5")]);
        assert_eq!(i8::deserialize(PathDeserializer::new(&p)).unwrap(), -5);

        let p = params(&[("value", "32767")]);
        assert_eq!(i16::deserialize(PathDeserializer::new(&p)).unwrap(), 32767);

        let p = params(&[("value", "2147483647")]);
        assert_eq!(
            i32::deserialize(PathDeserializer::new(&p)).unwrap(),
            2147483647
        );

        let p = params(&[("value", "255")]);
        assert_eq!(u8::deserialize(PathDeserializer::new(&p)).unwrap(), 255);

        let p = params(&[("value", "18446744073709551615")]);
        assert_eq!(
            u64::deserialize(PathDeserializer::new(&p)).unwrap(),
            u64::MAX
        );

        let p = params(&[("value", "3.5")]);
        assert_eq!(f32::deserialize(PathDeserializer::new(&p)).unwrap(), 3.5);

        let p = params(&[("value", "x")]);
        assert_eq!(char::deserialize(PathDeserializer::new(&p)).unwrap(), 'x');
    }

    #[test]
    fn deserialize_remaining_primitives_succeed() {
        let p = params(&[("value", "-42")]);
        assert_eq!(i64::deserialize(PathDeserializer::new(&p)).unwrap(), -42);

        let p = params(&[("value", "-123456789012345678")]);
        assert_eq!(
            i128::deserialize(PathDeserializer::new(&p)).unwrap(),
            -123456789012345678
        );

        let p = params(&[("value", "65535")]);
        assert_eq!(
            u16::deserialize(PathDeserializer::new(&p)).unwrap(),
            u16::MAX
        );

        let p = params(&[("value", "4294967295")]);
        assert_eq!(
            u32::deserialize(PathDeserializer::new(&p)).unwrap(),
            u32::MAX
        );

        let p = params(&[("value", "340282366920938463463374607431768211455")]);
        assert_eq!(
            u128::deserialize(PathDeserializer::new(&p)).unwrap(),
            u128::MAX
        );

        let p = params(&[("value", "2.5")]);
        assert_eq!(f64::deserialize(PathDeserializer::new(&p)).unwrap(), 2.5);
    }

    #[test]
    fn deserialize_any_unit_and_newtype_struct_succeed() {
        let p = params(&[("value", "abc")]);
        let any = AnyString::deserialize(PathDeserializer::new(&p)).unwrap();
        assert_eq!(any, AnyString("abc".to_string()));

        let p = params(&[("value", "whatever")]);
        assert_eq!(<()>::deserialize(PathDeserializer::new(&p)).unwrap(), ());

        let p = params(&[("value", "whatever")]);
        assert_eq!(
            UnitStruct::deserialize(PathDeserializer::new(&p)).unwrap(),
            UnitStruct
        );

        let p = params(&[("value", "wrapped")]);
        assert_eq!(
            NewtypeStruct::deserialize(PathDeserializer::new(&p)).unwrap(),
            NewtypeStruct("wrapped".to_string())
        );
    }

    #[test]
    fn unsupported_path_methods_are_reported() {
        let p = params(&[("value", "abc")]);
        let err = ForceBytes::deserialize(PathDeserializer::new(&p)).unwrap_err();
        assert!(matches!(
            err,
            PathDeserializationError::UnsupportedType { name }
            if name.ends_with("ForceBytes")
        ));

        let p = params(&[("value", "abc")]);
        let err = ForceIdentifier::deserialize(PathDeserializer::new(&p)).unwrap_err();
        assert!(matches!(
            err,
            PathDeserializationError::UnsupportedType { name }
            if name.ends_with("ForceIdentifier")
        ));
    }

    #[test]
    fn deserialize_seq_and_tuple_struct_succeed() {
        let p = params(&[("a", "10"), ("b", "20")]);
        let vec_values = Vec::<u32>::deserialize(PathDeserializer::new(&p)).unwrap();
        assert_eq!(vec_values, vec![10, 20]);

        let p = params(&[("key", "42")]);
        let pairs = Vec::<(String, u32)>::deserialize(PathDeserializer::new(&p)).unwrap();
        assert_eq!(pairs, vec![("key".to_string(), 42)]);
    }

    #[test]
    fn value_deserializer_unsupported_paths_are_reported() {
        let p = params(&[("key", "42")]);
        let err =
            HashMap::<String, Vec<String>>::deserialize(PathDeserializer::new(&p)).unwrap_err();
        assert!(matches!(
            err,
            PathDeserializationError::UnsupportedType { .. }
        ));

        let p = params(&[("key", "42")]);
        let err =
            HashMap::<String, PairStruct>::deserialize(PathDeserializer::new(&p)).unwrap_err();
        assert!(matches!(
            err,
            PathDeserializationError::UnsupportedType { .. }
        ));

        let p = params(&[("key", "42")]);
        let err = HashMap::<String, UserPath>::deserialize(PathDeserializer::new(&p)).unwrap_err();
        assert!(matches!(
            err,
            PathDeserializationError::UnsupportedType { .. }
        ));
    }

    #[test]
    fn value_deserializer_key_and_plain_parse_errors_are_distinct() {
        let p = params(&[("key", "abc")]);
        let err =
            HashMap::<String, (String, u32)>::deserialize(PathDeserializer::new(&p)).unwrap_err();
        assert_eq!(
            err,
            PathDeserializationError::Message("array types are not supported".to_string())
        );

        let p = params(&[("key", "abc")]);
        let err = Vec::<(String, u32)>::deserialize(PathDeserializer::new(&p)).unwrap_err();
        assert_eq!(
            err,
            PathDeserializationError::ParseError {
                value: "abc".to_string(),
                expected_type: "u32",
            }
        );
    }

    #[test]
    fn value_deserializer_option_and_enum_succeed() {
        let p = params(&[("status", "Active")]);
        let enum_map = HashMap::<String, UnitEnum>::deserialize(PathDeserializer::new(&p)).unwrap();
        assert_eq!(enum_map.get("status"), Some(&UnitEnum::Active));

        let p = params(&[("maybe", "hello")]);
        let option_map =
            HashMap::<String, Option<String>>::deserialize(PathDeserializer::new(&p)).unwrap();
        assert_eq!(option_map.get("maybe"), Some(&Some("hello".to_string())));
    }
}
