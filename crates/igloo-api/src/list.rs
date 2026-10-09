//! Filters shared by list endpoints.

use std::fmt;
use std::str::FromStr;

use igloo_core::{ValidationErrors, Validator};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// The order of a list. Lists are oldest first unless asked otherwise.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ListOrder {
    /// By creation, oldest first.
    #[default]
    Oldest,
    /// By creation, newest first.
    Newest,
}

/// A value that names no phase or order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownValue(pub String);

impl fmt::Display for UnknownValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown value {:?}", self.0)
    }
}

impl std::error::Error for UnknownValue {}

impl FromStr for ListOrder {
    type Err = UnknownValue;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "oldest" => Ok(Self::Oldest),
            "newest" => Ok(Self::Newest),
            other => Err(UnknownValue(other.to_owned())),
        }
    }
}

/// The `phase` (repeatable) and `order` parameters of a list of resources with phases `P`.
///
/// Invariant: no phases selected means every phase.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhaseFilter<P> {
    phases: Vec<P>,
    order: ListOrder,
}

impl<P> Default for PhaseFilter<P> {
    fn default() -> Self {
        Self {
            phases: Vec::new(),
            order: ListOrder::default(),
        }
    }
}

impl<P: PartialEq> PhaseFilter<P> {
    /// Whether a resource at `phase` is selected.
    #[must_use]
    pub fn matches(&self, phase: &P) -> bool {
        self.phases.is_empty() || self.phases.contains(phase)
    }

    /// The selected items of `oldest_first`, in the requested order.
    #[must_use]
    pub fn apply<T>(&self, oldest_first: Vec<T>, phase: impl Fn(&T) -> P) -> Vec<T> {
        let mut items: Vec<T> = oldest_first
            .into_iter()
            .filter(|item| self.matches(&phase(item)))
            .collect();
        if self.order == ListOrder::Newest {
            items.reverse();
        }
        items
    }
}

impl<P: FromStr> TryFrom<Vec<(String, String)>> for PhaseFilter<P>
where
    P::Err: fmt::Display,
{
    type Error = ValidationErrors;

    /// Reads `phase` and `order` from query pairs; other pairs are ignored.
    fn try_from(params: Vec<(String, String)>) -> Result<Self, Self::Error> {
        let mut phases: Result<Vec<P>, String> = Ok(Vec::new());
        let mut order: Result<ListOrder, String> = Ok(ListOrder::default());
        for (key, value) in params {
            match key.as_str() {
                "phase" => {
                    phases = phases.and_then(|mut phases| {
                        phases.push(value.parse::<P>().map_err(|error| error.to_string())?);
                        Ok(phases)
                    });
                }
                "order" => {
                    order = value
                        .parse()
                        .map_err(|error: UnknownValue| error.to_string());
                }
                _ => {}
            }
        }
        let (phases, order) = Validator::new()
            .field("phase", phases)
            .field("order", order)
            .finish()?;
        Ok(Self { phases, order })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::change::ChangePhase;

    fn query(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn no_parameters_select_everything_oldest_first() {
        let filter = PhaseFilter::<ChangePhase>::try_from(query(&[])).expect("valid");
        assert_eq!(
            filter.apply(vec![1, 2, 3], |_| ChangePhase::Merged),
            [1, 2, 3]
        );
    }

    #[test]
    fn repeated_phases_select_any_of_them_and_newest_reverses() {
        let filter = PhaseFilter::<ChangePhase>::try_from(query(&[
            ("phase", "open"),
            ("phase", "closed"),
            ("order", "newest"),
        ]))
        .expect("valid");
        let phases = [ChangePhase::Open, ChangePhase::Merged, ChangePhase::Closed];
        assert_eq!(filter.apply(vec![0, 1, 2], |n| phases[*n]), [2, 0]);
    }

    #[test]
    fn unknown_values_are_reported_by_parameter() {
        let error =
            PhaseFilter::<ChangePhase>::try_from(query(&[("phase", "done"), ("order", "up")]))
                .expect_err("invalid");
        let fields: Vec<&str> = error.fields().map(|(field, _)| field).collect();
        assert_eq!(fields, ["order", "phase"]);
    }
}
