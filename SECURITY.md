# Security policy

## Supported versions

Ripscope is pre-1.0. Security fixes land on `main` and in the next release; only
the latest release is supported.

## Reporting a vulnerability

Please report suspected vulnerabilities privately through GitHub's
[private vulnerability reporting](https://github.com/pederbe/ripscope/security/advisories/new)
(repository **Security** tab → **Report a vulnerability**). Do not open a public
issue for a suspected vulnerability.

If that form is unavailable, use the [maintainer's contact form](https://pederbe.dev/#contact)
to request a private channel. Do not include exploit details in the first message.

Include the ripscope version or commit, your platform, and a minimal reproduction.
The maintainer aims to acknowledge reports within a week. Once a fix is available, the
advisory is published with credit unless you ask otherwise.

## Scope

Ripscope reads untrusted source files and writes a local index. Relevant reports
include, for example:

- crashes, hangs or unbounded resource use triggered by crafted source files,
  file lists or index files;
- reading or writing files outside the selected sources, include directories and
  index location;
- index corruption that is reported as a healthy result.

The indexer and query executable do not execute source code, run a compiler or
preprocessor, or access the network. Optional install scripts download release
archives; include the script version and download source in reports about them.
