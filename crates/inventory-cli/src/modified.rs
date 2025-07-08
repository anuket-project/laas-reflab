use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fmt,
};

use crate::prelude::InventoryError;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Default, Clone)]
pub struct ModifiedFields {
    fields: HashSet<String>,
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
        if self.fields.contains(field_name) {
            // both maps must have the key if it’s in `fields`
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
        if self.fields.contains(&field_name) {
            return Err(InventoryError::FieldAlreadyModified(field_name));
        }

        self.fields.insert(field_name.clone());
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
        for original in other.fields {
            let merged_name = format!("{}.{}", prefix, original);
            if self.fields.contains(&merged_name) {
                return Err(InventoryError::FieldAlreadyModified(merged_name));
            }
            // unwraps here are safe because `other` guaranteed to
            // have those keys in its `old` and `new` maps
            let old_val = other.old.get(&original).unwrap().clone();
            let new_val = other.new.get(&original).unwrap().clone();

            self.fields.insert(merged_name.clone());
            self.old.insert(merged_name.clone(), old_val);
            self.new.insert(merged_name.clone(), new_val);
        }
        Ok(())
    }
}

impl fmt::Display for ModifiedFields {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // split into top-level vs nested
        let mut top = Vec::new();
        let mut nested: HashMap<String, Vec<(String, String, String)>> = HashMap::new();

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
                nested
                    .entry(prefix.to_string())
                    .or_default()
                    .push((name.to_string(), old, new));
            }
        }

        // sort top-level fields
        top.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, old, new) in top {
            writeln!(
                f,
                "    {}: {} {} {}",
                name,
                old.red(),
                "→".yellow(),
                new.green()
            )?;
        }

        // sort prefixes and their entries
        let mut prefixes: Vec<_> = nested.keys().cloned().collect();
        prefixes.sort();
        for prefix in prefixes {
            writeln!(f, "    {}:", prefix)?;
            let mut items = nested.remove(&prefix).unwrap();
            items.sort_by(|a, b| a.0.cmp(&b.0));
            for (name, old, new) in items {
                writeln!(
                    f,
                    "      {}: {} {} {}",
                    name,
                    old.red(),
                    "→".yellow(),
                    new.green()
                )?;
            }
        }

        Ok(())
    }
}
