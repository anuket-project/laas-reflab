# Overview

The [`users`] crate is responsible for talking to IPA.
[`IPA`] is a centralized user management system that is used to authenticate users and manage their permissions.
It is also used to manage the VPN and other services that are used to connect to the lab.

Most of the types in `ipa.rs` represent the data that is returned from the IPA API. Then each of the
functions hit the IPA API and return data for use in the rest of the application. At this point, the
only authorization and authentication is performed through IPA. This is why [`laas-reflab`]
should only be exposed in its current state to a trusted network/frontend service.

> _"I'm sorry this exists, I'll do better next time." ~ Raven_

[`users`]: self
[`IPA`]: https://www.freeipa.org/page/Main_Page
[`laas-reflab`]: ../laas_reflab/index.html
