use crate::prelude::{
    InventoryError, ModifiedFields, Reportable, Switch, SwitchPort, SwitchYaml, SwitchportReport,
    switch, switchport,
};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
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

    pub async fn execute_modified(&self, pool: &PgPool) -> Result<(), InventoryError> {
        if let SwitchReport::Modified {
            switch_yaml,
            switchport_reports,
            ..
        } = self
        {
            switch::update_switch_by_name(pool, switch_yaml).await?;

            for switchport_report in switchport_reports {
                switchport_report.execute(pool).await?;
            }

            Ok(())
        } else {
            Err(InventoryError::InvalidReportType {
                expected: "Modified",
                actual: self.report_name(),
            })
        }
    }

    pub async fn execute_created(&self, pool: &PgPool) -> Result<(), InventoryError> {
        if let SwitchReport::Created { switch_yaml } = self {
            println!("Creating switch... {}", switch_yaml.name);
            switch::create_switch(pool, switch_yaml).await?;

            println!("Creating switchports for switch... {}", switch_yaml.name);
            for switchport_name in &switch_yaml.switchports {
                println!("Creating switchport... {}", switchport_name);
                switchport::create_switchport(pool, &switch_yaml.name, switchport_name).await?;
            }

            Ok(())
        } else {
            Err(InventoryError::InvalidReportType {
                expected: "Created",
                actual: self.report_name(),
            })
        }
    }

    pub async fn execute_removed(&self, pool: &PgPool) -> Result<(), InventoryError> {
        if let SwitchReport::Removed {
            db_switch,
            removed_switchports,
        } = self
        {
            println!("Removing switchports from switch... {}", db_switch.name);

            for switchport in removed_switchports {
                println!("Removing switchport... {}", switchport.name);
                switchport::delete_switchport(pool, &db_switch.name, &switchport.name).await?;
            }
            println!("Removing switch... {}", db_switch.name);

            switch::delete_switch_by_name(pool, db_switch).await
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
            SwitchReport::Created { .. } => 0,
            SwitchReport::Modified { .. } => 1,
            SwitchReport::Removed { .. } => 2,
            SwitchReport::Unchanged => 3,
        }
    }

    async fn execute(&self, pool: &PgPool) -> Result<(), InventoryError> {
        match self {
            SwitchReport::Created { .. } => self.execute_created(pool).await,
            SwitchReport::Modified { .. } => self.execute_modified(pool).await,
            SwitchReport::Removed { .. } => self.execute_removed(pool).await,
            SwitchReport::Unchanged => self.execute_unchanged(),
        }
    }
}

impl fmt::Display for SwitchReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SwitchReport::Created { switch_yaml } => {
                write!(
                    f,
                    " {} {}",
                    "Created Switch:".green().bold(),
                    switch_yaml.name.green(),
                )?;

                for switchport in &switch_yaml.switchports {
                    writeln!(f, "  - {}", switchport.green())?;
                }

                Ok(())
            }
            SwitchReport::Removed {
                db_switch,
                removed_switchports,
            } => {
                write!(
                    f,
                    " {} {}",
                    "Removed Switch:".red().bold(),
                    db_switch.name.red()
                )?;

                for switchport in removed_switchports {
                    writeln!(f, "  - {}", switchport.name.red())?;
                }

                Ok(())
            }
            SwitchReport::Modified {
                switch_yaml,
                fields,
                switchport_reports,
            } => {
                writeln!(
                    f,
                    " {} {}",
                    "Modified Switch:".yellow().bold(),
                    switch_yaml.name.yellow()
                )?;

                let db_report = fields.to_string();
                for line in db_report.lines() {
                    writeln!(f, "{}", line)?;
                }

                for switchport_report in switchport_reports {
                    writeln!(f, "{}", switchport_report)?;
                }

                Ok(())
            }

            // ignore unchanged
            _ => Ok(()),
        }
    }
}
