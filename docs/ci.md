# Homebody in a build pipeline

`homebody ci` runs an agent protected when nobody is at the keyboard. It is the same fence, gate, rules and signed log as the app. The differences are the ones a machine needs:

- **Nothing is ever asked.** A place that is not trusted is refused at once.
- **The places a run needs are named up front** and forgotten when it ends.
- **The project is brought in as a copy.** The agent works there; with `--apply` its changes are put back.
- **The run ends with a summary**, and with `--report` a folder of signed evidence.
- **The exit code is the command's own**, or 3 when something was refused and you asked for that to fail the job.

```bash
homebody ci [options] <app> -- <command...>
```

| Option | What it does |
|---|---|
| `--project <folder>` | Bring this folder in as a copy and run there. The copy is deleted afterwards. |
| `--apply` | Afterwards, put the changes back into the folder. |
| `--keep-copy` | Leave the copy in place afterwards. |
| `--trust <place>` | Also trust this place for this run. Repeatable. |
| `--rules <file>` | Also trust the places a file names: `{"trust": ["registry.npmjs.org"]}`. |
| `--env <NAME>` | Hand this variable to the command (an API key, say). Nothing else of the job's environment is passed on. |
| `--report <folder>` | Save the signed log, a table, the rules, the rules as Cedar policies, the compliance mapping and a summary there. |
| `--fail-on-blocked` | Exit with 3 if anything was refused. |

A place named with `--trust` or `--rules` that already has a rule on the machine is left as it is: a place blocked there stays blocked.

## Once per machine

The fence needs an administrator once. On a build machine nobody can click "approve", so print what setup does and run it:

```bash
homebody setup --print | sudo bash
```

The script is the same one the app shows before you approve it. It can be run again safely. On Windows there is nothing to set up.

The agent has to be installed for the whole machine, not inside a home folder: a protected program cannot enter one.

## GitHub Actions

```yaml
jobs:
  agent:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: devesh16145/Homebody/ci/action@main
        with:
          add: "My agent=/usr/local/bin/my-agent"   # only for a program Homebody has no preset for
      - run: >
          homebody ci --project . --apply --rules .homebody.json --env MY_API_KEY
          --report homebody-report --fail-on-blocked
          my-my-agent -- my-agent --task "update the changelog"
        env:
          MY_API_KEY: ${{ secrets.MY_API_KEY }}
      - uses: actions/upload-artifact@v4
        if: always()
        with:
          name: homebody-report
          path: homebody-report
```

The summary also appears on the run's page (it is written to `GITHUB_STEP_SUMMARY`).

## Checking a report later

```bash
homebody verify homebody-report
```

That needs no running app and no access to the machine the run happened on. It prints the key that signed the log; a log signed by a different key was made somewhere else.

## What has been run

On this project's own Linux build machine: the action from nothing, a clean run with a report that verifies, and a run with a refusal exiting 3. On macOS: the same by hand. On Windows: a clean run. A Linux runner is the cheapest place to use it.
