# Overview

The [`dal`] (Database Access Layer) workspace defines models that interact with
the SQL database. This workspace is essentially a lightweight ORM.

Models (think tables in a database) are just structs that
implement the [`DBTable`] trait. Models are defined in
the [`models`] workspace and encapsulate their own data and migrations.

Database operations are run using [`tokio-postgres`].

> 🚨 **Warning:** If you are retrieving data from the database to be sent as a JSON web response from within `liblaas`, you should use the `Blob` types defined in [`liblaas::web::api`]. These are safe versions/views of types in the [`models`] crate that derive [`Serialize`] and [`Deserialize`] and omit any sensitive fields.

## Usage

### Creating a New Row

This example demonstrates how to insert a new row into the database using [`NewRow`]. It assumes you have a model called `User` that implements the [`DBTable`] trait.

> **Note:** In almost all cases within [`laas-reflab`], model primary keys are [`UUID`]'s'.

```rust
use dal::{
    new_client,
    AsEasyTransaction,
    DBTable,
    NewRow,
    ID,
},
use models::User;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
let mut client = dal::new_client().await?;
let mut transaction = client.easy_transaction().await?;

    let user = User {
        id: ID::new(), // automatically generate a new UUID for this user
        name: "John Doe".to_string(),
        email: "john.doe@example.com".to_string(),
    };

    let new_user = NewRow::new(user);
    new_user.insert(&mut transaction).await?;

    transaction.commit().await?;

    Ok(())

}
```

### Fetching a Row

This example fetches a row representing a [`Host`]
by its primary key using the [`AsEasyTransaction`]
trait and [`ExistingRow`] given a uuid.

```rust
use dal::{AsEasyTransaction, DBTable, ExistingRow, ID, new_client};
use models::inventory::Host;
use std::str::FromStr;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
  let host_uuid = ID::from_str("00000000-0000-0000-0000-000000000000")?;

  let mut client = new_client().await.unwrap();
  let mut transaction = client.easy_transaction().await?;

  // fetch the host directly with it's uuid
  let host: ExistingRow<Host> = Host::get(&mut transaction, host_uuid).await?;

  transaction.commit().await?;

  // do something with the host
  println!("Host: {:?}", host);

  Ok(())
}
```

### Updating a Row

This example shows how to fetch a row from the database and update it.
It uses [`ExistingRow`] to handle the fetched row.

```rust
use dal::{AsEasyTransaction, DBTable, ExistingRow, ID, new_client};
use models::User;
use std::str::FromStr;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = dal::new_client().await?;
    let mut transaction = client.easy_transaction().await?;

    // assume we have the user's uuid
    let user_id = ID::from_str("00000000-0000-0000-0000-000000000000")?;

    // fetch the user
    let existing_user: ExistingRow<User> = User::get(&mut transaction, user_id).await?;

    // update some fields of the user
    let mut user = existing_user.into_inner();
    user.email = "new.email@example.com".to_string();

    // update the user in the database
    let updated_user = ExistingRow::from_existing(user);
    updated_user.update(&mut transaction).await?;

    transaction.commit().await?;

    Ok(())
}
```

### Deleting a Row

This example illustrates how to delete a row from the database using the [`ExistingRow`]'s [`delete()`](<ExistingRow::delete()>) method.

```rust
use dal::{AsEasyTransaction, DBTable, ExistingRow, ID, new_client};
use models::User;
use std::str::FromStr;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = dal::new_client().await?;
    let mut transaction = client.easy_transaction().await?;

    // assume we have the user's ID to delete
    let user_id = ID::from_str("00000000-0000-0000-0000-000000000000")?;

    // fetch the user to ensure it exists before deleting
    let existing_user: ExistingRow<User> = User::get(&mut transaction, user_id).await?;

    // delete the user
    existing_user.delete(&mut transaction).await?;

    transaction.commit().await?;

    Ok(())
}
```

[`Host`]: ../models/inventory/struct.Host.html
[`laas-reflab`]: ../laas_reflab/index.html
[`UUID`]: uuid::Uuid
[`models`]: ../models/index.html
[`dal`]: self::dal
[`tokio-postgres`]: https://docs.rs/tokio-postgres
[`Deserialize`]: serde::Deserialize
[`Serialize`]: serde::Serialize
[`DBTable`]: DBTable
[`NewRow`]: NewRow
[`ExistingRow`]: ExistingRow
[`AsEasyTransaction`]: AsEasyTransaction
[`ID`]: ID
[`liblaas::web::api`]: ../liblaas/web/api/index.html
