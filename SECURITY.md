# Security policy

Walaru is pre-release software. Security fixes are applied to the current `main` branch; there are no supported release branches yet.

Do not report vulnerabilities in a public issue. Use this repository's [private vulnerability reporting form](https://github.com/P4suta/walaru/security/advisories/new). Include the affected revision, platform, impact, reproduction steps, and any proposed mitigation. Avoid attaching real secrets or private source code.

Maintainers will acknowledge a report within seven days, validate and scope it, coordinate a fix, and publish an advisory when disclosure is safe. Exact timing depends on severity and dependency coordination.

Security-sensitive areas include untrusted workspace execution, local transport permissions, artifact discovery, command argument boundaries, captured values and inputs, redaction, response/log bounds, archive construction, and replay completeness claims.
