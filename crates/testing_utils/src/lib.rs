use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use dal::{DBTable, EasyTransaction, ExistingRow, FKey, NewRow};
use eui48::MacAddress;
use macaddr::MacAddr6;
use once_cell::sync::Lazy;
use prop::collection::{hash_map, vec};
use prop::sample::SizeRange;
use proptest::prelude::*;
use serde_json::{Map, Value};
use sqlx::{postgres::PgPoolOptions, PgPool};
use std::collections::HashMap;
use std::hash::Hash;

// This magic library called `ctor` somehow runs before any other step in the test binary
// we use it to install color_eyre for prettier panic messages (we can't do this in each test
// because they run in parallel)
#[ctor::ctor]
fn init() {
    color_eyre::install();
}

// will only be used when using sqlx in tests.
static TEST_POOL: Lazy<PgPool> = Lazy::new(|| {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let max_connections = std::env::var("MAX_CONNECTIONS")
        .unwrap_or_else(|_| "10".to_string())
        .parse::<u32>()
        .expect("env var MAX_CONNECTIONS must be a number");

    PgPoolOptions::new()
        .max_connections(max_connections)
        .connect_lazy(&url)
        .expect("Failed to create pool")
});

pub fn test_pool() -> PgPool {
    TEST_POOL.clone()
}

/// Generates a random [`DateTime<Utc>`] within a reasonable range.
pub fn datetime_utc_strategy() -> impl Strategy<Value = DateTime<Utc>> {
    (0i64..=4102444800i64) // timestamps from 1970-01-01 to 2100-01-01
        .prop_map(|timestamp| DateTime::from_timestamp(timestamp, 0).unwrap())
}

/// Generates a [`NaiveDateTime`] within a reasonable range.
pub fn naive_datetime_strategy() -> impl Strategy<Value = NaiveDateTime> {
    // define a range for valid timestamps (e.g., from 1970 to 2100)
    let min_timestamp = 0_i64; // January 1, 1970
    let max_timestamp = 4_102_444_800_i64; // January 1, 2100

    // generate a timestamp (seconds since Unix epoch) and nanoseconds
    (min_timestamp..max_timestamp, 0..1_000_000_000u32).prop_map(|(secs, nsecs)| {
        // use `from_timestamp` to create a `DateTime<Utc>`, then convert to `NaiveDateTime`
        DateTime::from_timestamp(secs, nsecs)
            .map(|dt| dt.naive_utc()) // convert to `NaiveDateTime`
            .unwrap_or_else(|| {
                // fallback to a default value if the timestamp is invalid
                NaiveDateTime::default()
            })
    })
}

/// Generates a random [`eui48::MacAddress`] for property testing.
pub fn mac_address_strategy_eui48() -> impl Strategy<Value = eui48::MacAddress> {
    (any::<u64>()).prop_map(|value| {
        let bytes = value.to_be_bytes();
        MacAddress::new([bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7]])
    })
}

/// Generates a random [`macaddr::MacAddr6`] for property testing.
pub fn mac_addr6_strategy() -> impl Strategy<Value = MacAddr6> {
    (any::<u64>()).prop_map(|value| {
        let bytes = value.to_be_bytes();
        MacAddr6::new(bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7])
    })
}

/// Generates a random [`mac_address::MacAddress`] for property testing.
pub fn mac_address_strategy() -> impl Strategy<Value = mac_address::MacAddress> {
    (any::<u64>()).prop_map(|value| {
        let bytes = value.to_be_bytes();
        mac_address::MacAddress::new([bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7]])
    })
}

/// Generates a random arbitrary [`serde_json::Value`] for property testing.
pub fn arb_json_value() -> impl Strategy<Value = Value> {
    // define a base "leaf" strategy: null, bool, number, or string.
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(|n| Value::Number(n.into())),
        // generate strings without escape sequences (workaround)
        "[^\\x00-\\x1F\\\\]*".prop_map(Value::String)
    ];
    // use `prop_recursive` to allow for arrays and objects that can nest JSON values.
    leaf.prop_recursive(3, 12, 5, |inner| {
        prop_oneof![
            // an array of JSON values.
            vec(inner.clone(), 0..10).prop_map(Value::Array),
            //convert the HashMap into a serde_json::Map.
            hash_map("[^\\x00-\\x1F\\\\]*", inner, 0..10).prop_map(
                |map: HashMap<String, Value>| {
                    Value::Object(map.into_iter().collect::<Map<String, Value>>())
                }
            )
        ]
    })
}

/// Generates a random arbitrary [`HashMap`] with keys of type `T` and values of type
/// [`serde_json::Value`]
///
/// Parameters:
/// range: the range of the number of key-value pairs in the map.
/// T: Type of the keys in the map.
pub fn arb_json_map<T>(
    range: impl Into<SizeRange>,
) -> impl Strategy<Value = HashMap<T, serde_json::Value>>
where
    T: Arbitrary + Eq + Hash,
{
    hash_map(any::<T>(), arb_json_value(), range)
}

/// Inserts a default model of type `T` into the database at the given id. Commonly used while
/// setting up database tests.
pub async fn insert_default_model_at<T: Default + DBTable>(
    id: FKey<T>,
    t: &mut EasyTransaction<'_>,
) -> Result<(), anyhow::Error> {
    let model = T::default().assign_new_id(id);
    let new_row = NewRow::new(model);
    new_row.insert(t).await?;
    Ok(())
}

/// This should only be used to get rid of a little boilerplate inside `proptest` macros since it
/// doesn't support the `#[tokio::test]` attribute macro.
#[macro_export]
macro_rules! block_on_runtime {
    ($($block:tt)+) => {{
        let runtime = ::tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
        runtime.block_on(async { $($block)+ })
    }};
}
