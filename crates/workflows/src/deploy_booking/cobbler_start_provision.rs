//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use common::prelude::tracing;
use dal::FKey;

use models::inventory::Host;
use serde::{Deserialize, Serialize};
use tascii::prelude::*;

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct CobblerStartProvision {
    pub host_id: FKey<Host>,
}

tascii::mark_task!(CobblerStartProvision);
impl Runnable for CobblerStartProvision {
    type Output = bool;

    fn run(
        &mut self,
        _context: &tascii::prelude::Context,
    ) -> Result<Self::Output, tascii::prelude::TaskError> {
        tracing::warn!("entered and returning from cobbler start provision");

        // currently a noop, could change if we handle wrangling
        // host power management to Cobbler
        Ok(true)
    }

    fn identifier() -> tascii::task_trait::TaskIdentifier {
        TaskIdentifier::named("CobblerStartProvisionTask").versioned(1)
    }

    fn retry_count(&self) -> usize {
        0
    }
}
