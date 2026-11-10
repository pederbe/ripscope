# Installers resolve the latest release

This supersedes the checkout-version selection in
[ADR 0014](0014-own-editor-integrations.md), "Ripscope ships its own editor
integrations, set up in one step". Installers and user installation instructions
follow only the latest release. Editor install commands resolve that release too,
even when their checkout is older, while leaving executables managed elsewhere
under their existing manager's control.

Release and editor checks serve candidate archives through a loopback mirror whose
`/latest` endpoint identifies the candidate. Testing an unpublished release needs
no user-facing version selector. Package recipes retain the release identity and
hashes required by their package managers.
