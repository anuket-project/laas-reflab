# Metrics

This workspace contains several pieces that are used to record and send metrics to a telegraf
instance that is configured with a socket listener.
The primary goal of metrics collection is to provide
a way to visualize and export data and events produced by tascii.

## Metrics

Currently the following metrics are collected:

- Host [`Provision`]
- [`Booking`] Creation

### Usage

You can very easily construct and send any metric like so:

```rust
use metrics::prelude::*;

fn main() {
  let booking = Booking::new(
    5.0,
    3,
    2,
    "example_user".to_string(),
    "example_project".to_string(),
  );

  let booking_message = MetricMessage::from(booking);

  // This returns a Result of Ok() or [`metrics::error::MetricsError`]
  MetricHandler::global_sender().send(booking_message).unwrap();
}

```

[`Booking`]: metrics::Booking
[`Provision`]: metrics::Provision

```

```

```

```
