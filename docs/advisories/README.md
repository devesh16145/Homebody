# Advisories

Every way around the gate or the fence that is found and fixed gets an entry here: what it was, which versions had it, how it was found, and the test that now covers it. Nothing is fixed quietly.

Entries are numbered `SAI-<year>-<number>` and listed newest first. The ones below were found by the project itself before any release; they are written up all the same, because the habit matters more than the audience.

| Id | Fixed in | What it was |
|---|---|---|
| SAI-2026-003 | unreleased, Nov 2026 | With looking inside on, a request that merely asked to switch to a web socket made the gate stop reading the rest of that connection, even when the server did not switch. A following request could then name another site or a switched-off address unchecked. Now nothing more is passed on until the server has answered, and what follows is read as what it is. Test: `nothing_more_goes_until_the_server_has_agreed_to_switch`. |
| SAI-2026-002 | unreleased, Nov 2026 | An address inside a place that was switched off could be reached by respelling it (a doubled slash, a dot segment, an escaped letter, different capitals). Addresses are now judged as written and as a forgiving server would read them. Tests: `another_spelling_of_a_switched_off_address_is_refused_too`, seven cases in the bypass corpus. |
| SAI-2026-001 | unreleased, Nov 2026 | Refusals made by the launch-time profile and the packet filter (a program connecting directly instead of through the gate) were enforced but not shown anywhere. They are now read from the system log and appear in the Feed. Not a way around, but a blind spot. |

## How to report

See [SECURITY.md](../../SECURITY.md).
