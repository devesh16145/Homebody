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

## Test inside a real editor

```bash
test/in-editor/run.sh
```

Gradle starts the editor the plugin is built against, with a throw-away profile and a throw-away project. A build made for this run carries `src/selfcheck`, which runs the plugin's own actions and presses the buttons of the dialogs they really show, against the Homebody running on this computer: it brings the project in for one agent, opens that agent protected in a Terminal window, has it change a file and takes the change back, answers one of Homebody's questions from the notification, blocks everything for a moment, and puts all of it back. A released build does not contain the self-check.

To read the built plugin against another JetBrains editor on this computer (JetBrains' own checker lists anything the plugin uses that is not there):

```bash
gradle verifyPlugin -PverifyAgainst=/Applications/DataGrip.app
```

## What has been run

- `test/in-editor/run.sh` in **IntelliJ IDEA Community 2024.2.4** on a Mac: 22 of 22, and nothing from the plugin in the editor's error log.
- JetBrains' checker against **DataGrip 2026.2.6**: compatible. The packaged plugin was then installed into that DataGrip and DataGrip started: it lists Homebody among its loaded plugins, the status bar entry exists and refreshes, and its log has no errors. The actions were not pressed there.
- One fault was found this way and fixed: a project opened through a link (`/tmp` on a Mac) was not recognised as already brought in.
- Not run: any other JetBrains editor, Linux. Windows is not supported yet.
