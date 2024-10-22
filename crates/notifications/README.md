# Overview

The [`notifications`] crate is a relatively small workspace within [`laas-reflab`].
It is responsible for sending notifications to users based on certain events.
Currently, it only supports sending notifications via email but should support
more driver implementations in the future. The email functionality depends on [`lettre`],
a mailer library for Rust and [`tera`] for templating email content.

## Template Files

All `.html` files (email templates) are stored in the `/templates` directory.
This directory is defined in the `config.yml` file, which is loaded at runtime.
See the [`config`] crate for configuration details.

## Styling

There are a collection of `.json` files within the `/templates/styles` directory.
These styles are inlined into the HTML at runtime with [`tera`] and are also
defined per project in the `config.yml` file. This is a hacky solution to avoid
repeating excessive inline styles for email templates because they do not
consistently support HTML `<style>` tags.

## Usage

[`tera`] will automatically pick up any `html` files in the `/templates` directory.
Just create a `.html` file for the desired template and use the `{{ }}` syntax
when defining variables. Here is an example `email_template.html`:

```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <title>{{ subject }}</title>
  </head>
  <body>
    <h1>{{ greeting }}</h1>
    <p>{{ message.paragraph }}</p>
    <footer>
      <p>Thank you for using our service.</p>
    </footer>
  </body>
</html>
```

Then in your Rust code, you can render the template with a context like so:

```rust
use tera::{Tera, Context};
use serde::Serialize;

// define a struct to hold the context data for your template
#[derive(Serialize)]
struct ExampleContext {
    subject: String,
    greeting: String,
}

fn main() -> tera::Result<()> {

    // initialize Tera and tell it where to find your templates
    // `lib.rs` does this initialization already.
    let tera = match Tera::new("templates/**/*.html") {
        Ok(t) => t,
        Err(_) => {}
    };

    // create an instance of your context struct with example data
    let context = ExampleContext {
        subject: "Your Booking has been Provisioned".to_string(),
        greeting: "Hello, John!".to_string(),
    };

    // create a new Tera context
    let mut tera_context = Context::new();

    // you can insert individual values
    tera_context.insert("user_name", &context.user_name);
    tera_context.insert("greeting", &context.greeting);

    // or insert valid JSON under a single key
    tera_context.insert("message", &json!({
      "paragraph": "Your booking has been provisioned and is ready for use."
    });

    // render the template with your context
    // `lib.rs` already has a render method you can use
    let rendered_template = tera.render("email_template.html", &tera_context)?;
    println!("{}", rendered_template);

    Ok(())
}

```

[`notifications`]: self
[`laas-reflab`]: ../laas_reflab/index.html
[`lettre`]: lettre
[`tera`]: tera
[`config`]: config
