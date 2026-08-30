# IntelliJ thin client

Import `walaru-external-tools.xml` as an IntelliJ External Tools toolset, or copy its three tool definitions into the IDE settings. Each action invokes the local `walaru` executable with a direct bounded JSON query and uses `$ProjectFileDir$` as the worktree. No project file or build configuration is required.

Set the command path to the packaged `bin/walaru` (or `walaru.exe` on Windows) if it is not on `PATH`. The IDE client deliberately contains no execution or storage logic; the CLI/daemon contract remains the single source of truth.
