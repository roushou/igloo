use std::collections::BTreeMap;
use std::fmt;

use super::ErrorCode;

/// Every validation failure of an input, by field path (`"limits.millicpus"`).
///
/// Invariant: a returned `ValidationErrors` is never empty.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ValidationErrors {
    fields: BTreeMap<String, Vec<String>>,
}

impl ValidationErrors {
    /// Errors containing a single message for `path`.
    #[must_use]
    pub fn single(path: impl Into<String>, message: impl Into<String>) -> Self {
        let mut errors = Self::default();
        errors.add(path, message);
        errors
    }

    /// Records `message` for `path`.
    pub fn add(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.fields
            .entry(path.into())
            .or_default()
            .push(message.into());
    }

    /// Records every error of `nested` under `prefix` (`"limits"` + `"millicpus"`).
    pub fn merge(&mut self, prefix: &str, nested: Self) {
        for (path, messages) in nested.fields {
            let full = match (prefix.is_empty(), path.is_empty()) {
                (true, _) => path,
                (false, true) => prefix.to_owned(),
                (false, false) => format!("{prefix}.{path}"),
            };
            self.fields.entry(full).or_default().extend(messages);
        }
    }

    /// Whether no error was recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// Field paths with their messages, ordered by path.
    pub fn fields(&self) -> impl Iterator<Item = (&str, &[String])> {
        self.fields
            .iter()
            .map(|(path, messages)| (path.as_str(), messages.as_slice()))
    }

    /// `Ok(value)` when no error was recorded, otherwise `Err(self)`.
    pub fn into_result<T>(self, value: T) -> Result<T, Self> {
        if self.is_empty() {
            Ok(value)
        } else {
            Err(self)
        }
    }
}

impl fmt::Display for ValidationErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for (path, messages) in &self.fields {
            for message in messages {
                if !first {
                    f.write_str("; ")?;
                }
                first = false;
                write!(f, "{path}: {message}")?;
            }
        }
        Ok(())
    }
}

impl std::error::Error for ValidationErrors {}

impl ErrorCode for ValidationErrors {
    fn code(&self) -> &'static str {
        "validation.invalid"
    }
}

/// Accumulates fallible field conversions and reports every failure at once.
///
/// ```
/// use igloo_core::{Validator, ValidationErrors};
///
/// let result: Result<(u8, u8), ValidationErrors> = Validator::new()
///     .field("a", "1".parse::<u8>())
///     .field("b", "x".parse::<u8>())
///     .finish();
/// assert!(result.is_err());
/// ```
#[must_use]
pub struct Validator<T> {
    values: Option<T>,
    errors: ValidationErrors,
}

impl Validator<()> {
    /// A validator with no fields yet.
    pub fn new() -> Self {
        Self {
            values: Some(()),
            errors: ValidationErrors::default(),
        }
    }
}

impl Default for Validator<()> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Validator<T> {
    /// Adds a field whose error is a single message.
    pub fn field<U, E: fmt::Display>(self, path: &str, result: Result<U, E>) -> Validator<T::Output>
    where
        T: Append<U>,
    {
        self.push(result.map_err(|error| ValidationErrors::single(path, error.to_string())))
    }

    /// Adds a field whose errors are themselves per-field, nested under `path`.
    pub fn nested<U>(self, path: &str, result: Result<U, ValidationErrors>) -> Validator<T::Output>
    where
        T: Append<U>,
    {
        self.push(result.map_err(|nested| {
            let mut errors = ValidationErrors::default();
            errors.merge(path, nested);
            errors
        }))
    }

    /// The converted values, or every recorded error.
    pub fn finish(self) -> Result<T, ValidationErrors> {
        match self.values {
            Some(values) if self.errors.is_empty() => Ok(values),
            _ => Err(self.errors),
        }
    }

    fn push<U>(mut self, result: Result<U, ValidationErrors>) -> Validator<T::Output>
    where
        T: Append<U>,
    {
        let values = match (self.values, result) {
            (Some(values), Ok(value)) => Some(values.append(value)),
            (_, Err(errors)) => {
                self.errors.merge("", errors);
                None
            }
            (None, Ok(_)) => None,
        };
        Validator {
            values,
            errors: self.errors,
        }
    }
}

/// Appends one element to a tuple; lets [`Validator`] return a flat tuple of field values.
pub trait Append<U> {
    /// The tuple with `U` appended.
    type Output;

    /// Appends `value`.
    fn append(self, value: U) -> Self::Output;
}

macro_rules! impl_append {
    ($($name:ident),*) => {
        impl<$($name,)* U> Append<U> for ($($name,)*) {
            type Output = ($($name,)* U,);

            #[allow(non_snake_case, reason = "tuple fields are named after their type parameters")]
            fn append(self, value: U) -> Self::Output {
                let ($($name,)*) = self;
                ($($name,)* value,)
            }
        }
    };
}

impl_append!();
impl_append!(A);
impl_append!(A, B);
impl_append!(A, B, C);
impl_append!(A, B, C, D);
impl_append!(A, B, C, D, E);
impl_append!(A, B, C, D, E, F);
impl_append!(A, B, C, D, E, F, G);
impl_append!(A, B, C, D, E, F, G, H);
impl_append!(A, B, C, D, E, F, G, H, I);
impl_append!(A, B, C, D, E, F, G, H, I, J);
impl_append!(A, B, C, D, E, F, G, H, I, J, K);
impl_append!(A, B, C, D, E, F, G, H, I, J, K, L);
impl_append!(A, B, C, D, E, F, G, H, I, J, K, L, M);
impl_append!(A, B, C, D, E, F, G, H, I, J, K, L, M, N);
impl_append!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_every_failing_field() {
        let mut nested = ValidationErrors::default();
        nested.add("x", "too big");
        let result = Validator::new()
            .field("a", "1".parse::<u8>())
            .field("b", "x".parse::<u8>())
            .field("c", "-1".parse::<u8>())
            .nested::<u8>("d", Err(nested))
            .finish();
        let errors = result.expect_err("three fields fail");
        let paths: Vec<&str> = errors.fields().map(|(path, _)| path).collect();
        assert_eq!(paths, ["b", "c", "d.x"]);
    }

    #[test]
    fn returns_a_flat_tuple_on_success() {
        let values = Validator::new()
            .field("a", "1".parse::<u8>())
            .field("b", "2".parse::<u16>())
            .field("c", "3".parse::<u32>())
            .finish();
        assert_eq!(values, Ok((1, 2, 3)));
    }
}
