//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

# Booking
The booking crate is given booking requests from the api crate, and communicates with the solve crate, <br> models crate, workflow crate, and Tascii to provide booking scheduling and start information.
## Functions
- Processes booking request
    - Booking validation
    - Requests to the solver
    - Updates booking information through models
    - Communicates booking to Tascii
    - Provides status updates on bookings
<details>
<summary><h2>Structure</h2></summary>
rework api
take inputs from the dashboard workflows and make templates given inputs
implement solve pick hosts to be used for provisioning (should be able to)

```
|
|__ src
  |__ booking.rs ()
  |
  |__ lib.rs ()

```
</details>
