# Overview

The [`workflows`] crate contains several modules that are responsible for the
registration, execution and cleanup of tasks like the provisioning of bookings,
setup and configuration of network resources like switches, VLAN's or
any other unit of work that needs to be executed as a result of a user request
from [`liblaas`].

As mentioned above, all workflows or "units of work" defined to be run and managed
by [`tascii`] implement a common [`AsyncRunnable`] trait. You can see [`tascii`]
documentation for more information on how [`tascii`] schedules and executes tasks.

Most workflows are tascii tasks. However, there are some workflows that are
simply functions that are exported and called by other modules within [`laas-reflab`].

[`workflows`]: self
[`AsyncRunnable`]: ../tascii/AsyncRunnable
[`tascii`]: tascii
[`laas-reflab`]: ../laas_reflab/index.html
[`liblaas`]: liblaas
