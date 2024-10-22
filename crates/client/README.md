# Overview

This workspace contains the interactive CLI client that allows
administrators to interact with an instance of `LibLaaS`.
The CLI is built using the [`inquire`](https://docs.rs/inquire/latest/inquire/) crate.
You can access the CLI by running `make cli` after setting up an
instance of `LibLaaS` while in the repository root.
See the [README](https://bitbucket.iol.unh.edu/projects/OST/repos/laas-reflab/browse/README.md)
for more information.

Administrative tasks include things like managing bookings, performing tests,
and re-provisioning hosts.

## Usage

Adding a new command to the CLI client is straightforward. Just add another
[`str`] in the list of commands in the `get_tasks()` function in `lib.rs`.
Then add a new match arm in `match task` in the same file to handle the new command.

```rust no_run
fn get_tasks(_session: &Server) -> Vec<&'static str> {
    vec![
        "Use database",
        "Use IPA",
        "Overrides",
        "My CLI Option", // add new command
        "Test LibLaaS",
        "Query",
    ]
}

// ...

match task.expect("Expected task array to be non-empty") {
    // ...other commands
    // add a new case for your command
    "My CLI Option" => {
      // do something
    },
    // ...
}
```

The above code will add a new option to the main menu of the TUI. If you would
like to nest your command in one of the existing options/categories
you have to find the parent function associated with that option and add your
entry there. Here is an example for the `Overrides` category:

```rust no_run
async fn overrides(mut session: &Server, tascii_rt: &'static Runtime) ->
  Result<(), anyhow::Error> {

  let mut client = new_client().await.expect("Expected to connect to db");
  let mut transaction = client
    .easy_transaction()
    .await
    .expect("Transaction creation error");

    match Select::new(
        "Select an override to apply:",
        vec![
            "override aggregate state",
            // other options
            "My Override",
        ],
    )
    .prompt(session)
    .unwrap()
    {
      "override aggregate state" => {
          // do something
      },
      // other options here
      "My Override" => {
          // do something
      },
    }
}
```

[`str`]: https://doc.rust-lang.org/std/primitive.str.html
