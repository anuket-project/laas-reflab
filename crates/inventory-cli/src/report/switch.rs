use crate::prelude::{
    InventoryError, ModifiedFields, Reportable, SortOrder, Switch, SwitchPort, SwitchYaml,
    SwitchportReport, switch, switchport,
};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};
use std::fmt;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub enum SwitchReport {
    Created {
        switch_yaml: SwitchYaml,
    },
    Modified {
        switch_yaml: SwitchYaml,
        fields: ModifiedFields,
        switchport_reports: Vec<SwitchportReport>,
    },
    Removed {
        db_switch: Switch,
        removed_switchports: Vec<SwitchPort>,
    },
    Unchanged,
}

impl SwitchReport {
    pub fn new_created(switch_yaml: SwitchYaml) -> Self {
        SwitchReport::Created { switch_yaml }
    }

    pub fn new_modified(switch_yaml: SwitchYaml, fields: ModifiedFields) -> Self {
        let switchport_reports = switch_yaml
            .clone()
            .switchports
            .iter()
            .map(|s| SwitchportReport::new_created(switch_yaml.clone(), s.to_string()))
            .collect();

        SwitchReport::Modified {
            switch_yaml,
            fields,
            switchport_reports,
        }
    }

    pub fn new_removed(db_switch: Switch, removed_switchports: Vec<SwitchPort>) -> Self {
        SwitchReport::Removed {
            db_switch,
            removed_switchports,
        }
    }

    pub fn is_unchanged(&self) -> bool {
        matches!(self, SwitchReport::Unchanged)
    }

    pub fn report_name(&self) -> &'static str {
        match self {
            SwitchReport::Created { .. } => "Created",
            SwitchReport::Modified { .. } => "Modified",
            SwitchReport::Removed { .. } => "Removed",
            SwitchReport::Unchanged => "Unchanged",
        }
    }

    pub fn item_name(&self) -> Option<&str> {
        match self {
            SwitchReport::Created { switch_yaml } => Some(&switch_yaml.name),
            SwitchReport::Modified { switch_yaml, .. } => Some(&switch_yaml.name),
            SwitchReport::Removed { db_switch, .. } => Some(&db_switch.name),
            SwitchReport::Unchanged => None,
        }
    }

    pub async fn execute_modified(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
    ) -> Result<(), InventoryError> {
        if let SwitchReport::Modified {
            switch_yaml,
            switchport_reports,
            ..
        } = self
        {
            switch::update_switch_by_name(transaction, switch_yaml).await?;

            for switchport_report in switchport_reports {
                switchport_report.execute(transaction).await?;
            }

            Ok(())
        } else {
            Err(InventoryError::InvalidReportType {
                expected: "Modified",
                actual: self.report_name(),
            })
        }
    }

    pub async fn execute_created(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
    ) -> Result<(), InventoryError> {
        if let SwitchReport::Created { switch_yaml } = self {
            switch::create_switch(transaction, switch_yaml).await?;

            for switchport_name in &switch_yaml.switchports {
                switchport::create_switchport(transaction, &switch_yaml.name, switchport_name)
                    .await?;
            }

            Ok(())
        } else {
            Err(InventoryError::InvalidReportType {
                expected: "Created",
                actual: self.report_name(),
            })
        }
    }

    pub async fn execute_removed(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
    ) -> Result<(), InventoryError> {
        if let SwitchReport::Removed {
            db_switch,
            removed_switchports,
        } = self
        {
            for switchport in removed_switchports {
                switchport::delete_switchport(transaction, &db_switch.name, &switchport.name)
                    .await?;
            }

            switch::delete_switch_by_name(transaction, db_switch).await
        } else {
            Err(InventoryError::InvalidReportType {
                expected: "Removed",
                actual: self.report_name(),
            })
        }
    }

    pub fn execute_unchanged(&self) -> Result<(), InventoryError> {
        if self.is_unchanged() {
            Ok(())
        } else {
            Err(InventoryError::InvalidReportType {
                expected: "Unchanged",
                actual: self.report_name(),
            })
        }
    }
}

impl Reportable for SwitchReport {
    fn is_unchanged(&self) -> bool {
        matches!(self, SwitchReport::Unchanged)
    }
    fn is_created(&self) -> bool {
        matches!(self, SwitchReport::Created { .. })
    }
    fn is_modified(&self) -> bool {
        matches!(self, SwitchReport::Modified { .. })
    }
    fn is_removed(&self) -> bool {
        matches!(self, SwitchReport::Removed { .. })
    }
    fn sort_order(&self) -> u8 {
        match self {
            SwitchReport::Created { .. } => SortOrder::Switch as u8,
            SwitchReport::Modified { .. } => SortOrder::Switch as u8 + 1,
            SwitchReport::Removed { .. } => SortOrder::Switch as u8 + 2,
            SwitchReport::Unchanged => SortOrder::Switch as u8 + 3,
        }
    }

    async fn execute(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
    ) -> Result<(), InventoryError> {
        match self {
            SwitchReport::Created { .. } => self.execute_created(transaction).await,
            SwitchReport::Modified { .. } => self.execute_modified(transaction).await,
            SwitchReport::Removed { .. } => self.execute_removed(transaction).await,
            SwitchReport::Unchanged => self.execute_unchanged(),
        }
    }
}

impl fmt::Display for SwitchReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SwitchReport::Created { switch_yaml } => {
                writeln!(
                    f,
                    "  {} {} [{}: {}]",
                    "+".green().bold(),
                    switch_yaml.name.bright_white().bold(),
                    "ports".dimmed(),
                    switch_yaml.switchports.len()
                )?;

                // Display ports in compact multi-column format (10 per line)
                if !switch_yaml.switchports.is_empty() {
                    for chunk in switch_yaml.switchports.chunks(10) {
                        write!(f, "      ")?;
                        writeln!(f, "{}", chunk.join(", "))?;
                    }
                }
                Ok(())
            }
            SwitchReport::Removed {
                db_switch,
                removed_switchports,
            } => {
                writeln!(
                    f,
                    "  {} {}",
                    "-".red().bold(),
                    db_switch.name.bright_white().bold()
                )?;
                writeln!(
                    f,
                    "      {} {} ports",
                    "removing".dimmed(),
                    removed_switchports.len()
                )?;
                Ok(())
            }
            SwitchReport::Modified {
                switch_yaml,
                fields,
                switchport_reports,
            } => {
                writeln!(
                    f,
                    "  {} {}",
                    "~".yellow().bold(),
                    switch_yaml.name.bright_white().bold()
                )?;

                let db_report = fields.to_string();
                for line in db_report.lines() {
                    writeln!(f, "{}", line)?;
                }

                for switchport_report in switchport_reports {
                    if !switchport_report.is_unchanged() {
                        writeln!(f, "      {}: {}", "port".dimmed(), switchport_report)?;
                    }
                }

                Ok(())
            }

            // ignore unchanged
            _ => Ok(()),
        }
    }
}
