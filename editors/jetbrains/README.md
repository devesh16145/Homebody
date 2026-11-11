# Homebody for JetBrains editors

IntelliJ IDEA, PyCharm, WebStorm and the other JetBrains editors, version 2024.2 or later, on macOS and Linux.

- **Status bar**: what Homebody is doing right now. Click it for the actions.
- **Questions**: when an agent wants to reach a new place, the question appears as a notification with Allow once, Always allow and Block.
- **Tools → Homebody**:
  - Open an agent protected, on this project (it works on a copy; a protected terminal opens)
  - Review what the agent changed, and put the changes into the project
  - Show recent activity
  - Block everything now

The plugin talks only to the Homebody app running on this computer, over the app's private socket.

## Build

Needs Java 21. Gradle downloads the IntelliJ development kit on the first build (about 1 GB).

```bash
gradle buildPlugin
```

The plugin is then in `build/distributions/`. Install it with Settings → Plugins → ⚙ → Install Plugin from Disk.

## What has been checked

It compiles against IntelliJ IDEA 2024.2 and passes the build's own plugin checks. It has not been run inside an editor on the machine it was written on, which has none installed. Windows is not supported yet.
