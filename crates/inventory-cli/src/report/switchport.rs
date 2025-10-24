use crate::prelude::{
    InventoryError, Reportable, SortOrder, Switch, SwitchPort, SwitchYaml, switchport,
};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};
use std::fmt;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub enum SwitchportReport {
    Created {
        switch_yaml: SwitchYaml,
        switchport_name: String,
    },
    Removed {
        db_switchport: SwitchPort,
        db_switch: Switch,
    },
    Unchanged {
        switch_yaml: SwitchYaml,
    },
}

impl SwitchportReport {
    pub fn new_created(switch_yaml: SwitchYaml, switchport_name: String) -> Self {
        SwitchportReport::Created {
            switch_yaml,
            switchport_name,
        }
    }

    pub fn new_removed(db_switchport: SwitchPort, db_switch: Switch) -> Self {
        SwitchportReport::Removed {
            db_switch,
            db_switchport,
        }
    }

    pub fn report_name(&self) -> &'static str {
        match self {
            SwitchportReport::Created { .. } => "Created",
            SwitchportReport::Removed { .. } => "Removed",
            SwitchportReport::Unchanged { .. } => "Unchanged",
        }
    }

    pub async fn execute_created(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
    ) -> Result<(), InventoryError> {
        if let SwitchportReport::Created {
            switch_yaml,
            switchport_name,
        } = self
        {
            switchport::create_switchport(transaction, &switch_yaml.name, switchport_name).await?;
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
        if let SwitchportReport::Removed {
            db_switch,
            db_switchport,
        } = self
        {
            switchport::delete_switchport(transaction, &db_switch.name, &db_switchport.name).await
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

impl Reportable for SwitchportReport {
    fn is_unchanged(&self) -> bool {
        matches!(self, SwitchportReport::Unchanged { .. })
    }
    fn is_created(&self) -> bool {
        matches!(self, SwitchportReport::Created { .. })
    }
    fn is_modified(&self) -> bool {
        false // DNE
    }
    fn is_removed(&self) -> bool {
        matches!(self, SwitchportReport::Removed { .. })
    }
    fn sort_order(&self) -> u8 {
        match self {
            SwitchportReport::Created { .. } => SortOrder::Switchport as u8,
            SwitchportReport::Removed { .. } => SortOrder::Switchport as u8 + 1,
            SwitchportReport::Unchanged { .. } => SortOrder::Switchport as u8 + 2,
        }
    }

    async fn execute(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
    ) -> Result<(), InventoryError> {
        match self {
            SwitchportReport::Created { .. } => self.execute_created(transaction).await,
            SwitchportReport::Removed { .. } => self.execute_removed(transaction).await,
            SwitchportReport::Unchanged { .. } => self.execute_unchanged(),
        }
    }
}

impl fmt::Display for SwitchportReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SwitchportReport::Created {
                switch_yaml: _,
                switchport_name,
            } => write!(f, "{}{}", "+".green(), switchport_name),
            SwitchportReport::Removed {
                db_switch: _,
                db_switchport,
            } => write!(f, "{}{}", "-".red(), db_switchport.name),

            // ignore unchanged
            _ => Ok(()),
        }
    }
}
