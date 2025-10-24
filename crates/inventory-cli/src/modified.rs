//! Tracking and displaying modified fields in inventory items
//!
//! This module provides the `ModifiedFields` type for tracking which fields
//! have changed between YAML definitions and database records, along with
//! their old and new values.

use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fmt,
};

use crate::prelude::InventoryError;

/// Tracks modified fields in an inventory item
///
/// This structure maintains three synchronized collections:
/// - `fields`: Ordered list of field names that have been modified (preserves insertion order)
/// - `old`: Map of field names to their old (database) values
/// - `new`: Map of field names to their new (YAML) values
///
/// The Display implementation provides color-coded output showing
/// old values in red, new values in green, with a yellow arrow between them.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Default, Clone)]
pub struct ModifiedFields {
    fields: Vec<String>,
    old: HashMap<String, String>,
    new: HashMap<String, String>,
}

impl ModifiedFields {
    pub(crate) fn new() -> Self {
        ModifiedFields::default()
    }

    /// If `field_name` was modified, returns `Some((&old_value, &new_value))`,
    /// otherwise `None`.
    pub fn get(&self, field_name: &str) -> Option<(&String, &String)> {
        if self.fields.iter().any(|f| f == field_name) {
            // both maps must have the key if it's in `fields`
            let old = self.old.get(field_name)?;
            let new = self.new.get(field_name)?;
            Some((old, new))
        } else {
            None
        }
    }

    pub(crate) fn modified<S, T, U>(
        &mut self,
        field_name: S,
        old: T,
        new: U,
    ) -> Result<(), InventoryError>
    where
        S: Into<String>,
        T: Into<String>,
        U: Into<String>,
    {
        let field_name = field_name.into();
        if self.fields.iter().any(|f| f == &field_name) {
            return Err(InventoryError::FieldAlreadyModified(field_name));
        }

        self.fields.push(field_name.clone());
        self.old.insert(field_name.clone(), old.into());
        self.new.insert(field_name.clone(), new.into());

        Ok(())
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// Merge in another [`ModifiedFields`], prefixing all of its field-names
    /// with `prefix` (e.g. `ipmi` → `ipmi.field_name`).
    ///
    /// Returns an `Err(InventoryError::FieldAlreadyModified(_))` if any
    /// of the prefixed names collide with fields already in `self`.
    pub(crate) fn merge(
        &mut self,
        prefix: impl AsRef<str>,
        other: ModifiedFields,
    ) -> Result<(), InventoryError> {
        let prefix = prefix.as_ref();
        for original in &other.fields {
            let merged_name = format!("{}.{}", prefix, original);
            if self.fields.iter().any(|f| f == &merged_name) {
                return Err(InventoryError::FieldAlreadyModified(merged_name));
            }
            // unwraps here are safe because `other` guaranteed to
            // have those keys in its `old` and `new` maps
            let old_val = other.old.get(original).unwrap().clone();
            let new_val = other.new.get(original).unwrap().clone();

            self.fields.push(merged_name.clone());
            self.old.insert(merged_name.clone(), old_val);
            self.new.insert(merged_name.clone(), new_val);
        }
        Ok(())
    }
}

impl fmt::Display for ModifiedFields {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // split into top-level vs nested, preserving insertion order
        let mut top = Vec::new();
        let mut nested: HashMap<String, Vec<(String, String, String)>> = HashMap::new();
        let mut prefix_order = Vec::new();

        for key in &self.fields {
            let (prefix, name) = if let Some(idx) = key.find('.') {
                (&key[..idx], &key[idx + 1..])
            } else {
                ("", key.as_str())
            };
            let old = self.old.get(key).unwrap().clone();
            let new = self.new.get(key).unwrap().clone();

            if prefix.is_empty() {
                top.push((name.to_string(), old, new));
            } else {
                let prefix_string = prefix.to_string();
                if !prefix_order.contains(&prefix_string) {
                    prefix_order.push(prefix_string.clone());
                }
                nested
                    .entry(prefix_string)
                    .or_default()
                    .push((name.to_string(), old, new));
            }
        }

        // Display top-level fields in insertion order (no sorting)
        for (name, old, new) in top {
            writeln!(f, "      {}: {} {} {}", name.dimmed(), old.red(), "->".dimmed(), new.green())?;
        }

        // Display nested fields in insertion order (no sorting)
        for prefix in prefix_order {
            writeln!(f, "      {}:", prefix.dimmed())?;
            let items = nested.get(&prefix).unwrap();
            for (name, old, new) in items {
                writeln!(f, "        {}: {} {} {}", name.dimmed(), old.red(), "->".dimmed(), new.green())?;
            }
        }

        Ok(())
    }
}
