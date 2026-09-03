# Translating Concat

Concat's interface is built in Slint, and Slint's `@tr()` mechanism produces
standard gettext `.po` files, the format translators already know how to
work with. The string catalogue is being wrapped for it; when it lands, this
file carries the instructions for adding a language: where the `.po` files
live, how to regenerate the template, and how a translation is reviewed
before it ships. Watch this file and the changelog.
