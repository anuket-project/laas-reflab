# Overview

The [`models`] crate is primarily a collection of definitions of models that
implement [`DBTable`] from the [`dal`] crate. Models are just tables in a SQL
database, each field is a column, and each row is an instance of the model/struct.
See the [`dal`] documentation for more information on how to actually query and mutate
instances of models in the database.

## Usage

The following shows how to define a new model and its migrations. Generally you
should only create a new model after quite a bit of thought and
deliberation (and hopefully communication), as migrations and database
state can get out of hand quickly. It is also likely there is already a
model or set of models that can be leveraged to solve the problem
related to your use case.

1. **Define Your Struct**: Define your struct with fields that represent
   the columns in your desired database table.

   ```rust
   use serde::{Serialize, Deserialize};
   use dal::{DBTable, ExistingRow, NewRow};

   #[derive(Serialize, Deserialize, Debug)]
   pub struct MyModel {
       pub id: i32,
       pub name: String,
       // other fields...
   }
   ```

      <br/>

2. **Implement the DBTable Trait**: Implement the [`DBTable`] trait for your model. [`table_name()`],
   [`to_rowlike()`], [`id()`], [`from_row()`] and [`migrations()`] are all required methods.

   ```rust
   impl DBTable for MyModel {
      fn table_name() -> &'static str {
          "my_models"
      }

      fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
          // conversion from a database row to your model
          // `ExistingRow` is defined in `dal` alongside `DBTable`
      }

      fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, Error> {
         // conversion from your model to a database row
      }

      fn id(&self) -> ID {
          self.id.into_id()
      }

      fn migrations() -> Vec<Migration> {
          // this will be explained in the next step
      }
   }
   ```

  <br/>

3.  **SQL Migrations**: Define SQL migration(s) for your model if you are creating a new table or modifying an existing one.
    Use the [`Migration`] and [`Apply`] types to define your migration(s).

    ```rust
    impl  {
        fn migrations() -> Vec<Migration> {
            vec![
                Migration {
                    unique_name: "create_my_models_table",
                    description: "Creates the my_models table",
                    apply: Apply::SQL("CREATE TABLE my_models
                             (id SERIAL PRIMARY KEY, name TEXT NOT NULL);".to_string()),
                   depends_on: vec![],
                },
                // you can define multiple migrations
            ]
        }
    }
    ```

    <br/>

4.  **Register Migrations**: Ensure your migrations are registered by calling the `inventory::submit!` macro on an instance of
    [`Migrate`] that wraps your migrations within a global scope.
    We use the [`inventory`] crate to collect all migrations and apply them to the database during startup.

    ```rust
     inventory::submit! { Migrate::new(MyModel::migrations) }
    ```

    <br/>

5.  **Use Your Model**: With your model and migrations defined,
    you can now use it within [`laas-reflab`].
    This includes performing CRUD operations through the methods provided by the [`DBTable`]
    trait and any additional custom `impl` blocks you define on your struct.

[`DBTable`]: dal::DBTable
[`laas-reflab`]: ../laas_reflab/index.html
[`Migration`]: dal::Migration
[`Apply`]: dal::Apply
[`table_name()`]: dal::DBTable::table_name
[`to_rowlike()`]: dal::DBTable::to_rowlike
[`id()`]: dal::DBTable::id
[`from_row()`]: dal::DBTable::from_row
[`migrations()`]: dal::DBTable::migrations
[`models`]: self
[`dal`]: dal
[`tokio-postgres`]: https://docs.rs/tokio-postgres
[`inventory`]: ../inventory/index.html
[`Migrate`]: dal::Migrate
