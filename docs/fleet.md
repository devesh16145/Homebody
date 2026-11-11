# Homebody for an organisation: the fleet server

Homebody keeps everything on the computer it runs on. A fleet is the one exception, and only by joining. An organisation runs its own server, `homebody-fleet`, and a computer that joins it:

- **takes the organisation's rules**, which then come before the person's own;
- **sends its log there**, record by record, exactly as it was written.

Nothing is hosted by anyone else. The server is one program with a folder of plain files next to it.

## What can and cannot be trusted

- **The rules cannot be forged on the way.** They are signed with the fleet's key. A computer pins that key when it joins and applies nothing that is not signed with it, and it never goes back to an older version.
- **The log cannot be changed on the server unnoticed.** Each record is signed by the computer that wrote it and chained to the one before. The server keeps a piece only if every record in it checks out and continues the last one; anyone it hands the log to can check the same.
- **The server can see what joined computers' AI apps reached.** That is what joining means, and the app says so before the Join button. It sees places, amounts and decisions, never content.
- **The server can make rules stricter or looser for joined computers.** A rule from the organisation that trusts a place overrides a person's own block of it. The place then shows in the app with the organisation's name on it.

## Run it

```bash
homebody-fleet --data /var/lib/homebody-fleet --name "Example Org" \
  --listen 0.0.0.0:8470 --tls-cert fleet.pem --tls-key fleet.key \
  --public-url https://fleet.example.org:8470
```

On the first start it prints a password for `admin`, once. Sign in at the public address and change it under People.

Computers only join over HTTPS (or on the same machine, for trying it out). Give the server a certificate as above, or put it behind a web server that has one. If the certificate is from your own authority, computers join with `--ca <file>`.

## Join a computer

In the console, **Joining → Make code**. A code has a team, a number of computers and an expiry. It is shown once.

On the computer: **Homebody → Settings → Organisation → Join one…**, or

```bash
homebody fleet join https://fleet.example.org:8470 <code>
homebody fleet            # how it is going
homebody fleet leave      # the person can always leave; the console then shows the computer going quiet
```

For managed Macs, the same page offers a **management profile** (`.mobileconfig`) carrying the address and the code. Push it with Jamf, Kandji or any other system; Homebody on those Macs joins by itself. The profile sets two values and approves nothing else.

## The console

| Section | What it is | Who |
|---|---|---|
| Computers | Each joined computer, when it was last seen, whether its log is intact, whether it has the current rules | everyone |
| Activity | What the computers' apps reached and what was stopped, searchable; **Export evidence** | auditors, administrators |
| Rules | The organisation's rules; whether unknown places ask, are blocked, or are left to each person; whether protection can be switched off | auditors read, administrators change |
| Joining | Codes and management profiles | administrators |
| People | Who may sign in, and as what | administrators |
| Settings | The organisation's name, how long logs are kept, the fleet key | administrators |

**Roles.** Viewers see computers. Auditors also see activity and rules, and can export. Administrators can change everything. The last administrator cannot be demoted or switched off.

**Retention.** With a period set, older records are removed from the server once a day. The hash of the last record removed is kept, so what remains still checks link by link.

**The console's own log.** Who signed in, who changed the rules, who exported: written to a signed, chained log of its own, downloadable under Settings and included in every export.

## Evidence

**Activity → Export evidence** downloads a `.tar` for a period and, if you like, one team:

- `computers/<name>/log.jsonl` and `computer.json`: each computer's log as it sent it, and the key that signed it. Check one anywhere with `homebody verify computers/<name>`.
- `activity.csv`: the period as a table.
- `rules.json`: the rules with the fleet's signature.
- `compliance.json`: where this evidence fits in NIST SP 800-53 and OWASP's list for AI agents ([docs/compliance.md](compliance.md)).
- `console-log.jsonl`: the console's own log. Check it with `homebody verify console-log.jsonl`.

## Sign-in through your identity provider

Optional. OpenID Connect, authorization code flow with PKCE; RS256-signed statements only.

```bash
homebody-fleet … --oidc-issuer https://id.example.org --oidc-client-id homebody-fleet \
  --oidc-client-secret-file secret.txt --oidc-admin-group fleet-admins --oidc-auditor-group fleet-auditors \
  --oidc-default-role none --oidc-label "Example ID"
```

Register `<public-url>/auth/callback` as the redirect address. A person's role follows the groups the provider lists for them at each sign-in. With `--oidc-default-role none`, people in neither group are not let in.

## Provisioning (SCIM 2.0)

Optional. With `--scim-token-file <file>`, an identity provider can add people, change their role (`roles[0].value`: `admin`, `auditor` or `viewer`) and switch them off at `<public-url>/scim/v2/Users`, sending the secret as a bearer token. People made this way have no password here.

## What has been run

- The server's own tests: a real listener and a real signed log; joining, signed rules, piece-by-piece log acceptance with tampered, missing and foreign records refused; the three roles; export checked with the app's own verifier; retention; provisioning; and sign-in against a stand-in identity provider. They run on macOS and on Linux.
- By hand on one Mac: a scratch copy of the engine joined a local server, took its rules, refused what they block, and its log arrived intact; the console was used in a browser.
- **Not yet run:** against a real identity provider (Okta, Entra, Google), with a real management system, with more than one computer, or over the internet.
