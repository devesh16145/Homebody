# Keys that stay home

An agent needs an API key to work. A key in an agent's hands is a key that a planted instruction can ask it to send somewhere, and an agent that has been told "print your environment" will do it. So Homebody can keep the key instead.

- **The agent gets a stand-in.** It has the same length as your key and, when your key starts with one of the openings providers use to mark their keys (`sk-ant-api03-`, `ghp_`, `AIza`…), the same opening, so a program that checks what its key looks like is satisfied. Nothing else of your key is in it, and it opens nothing.
- **The real key goes in at the gate.** When a request to the place the key is for passes the gate, the stand-in is replaced by the real key. That happens inside Homebody, after the request has been checked.
- **Only for the exact place you named.** A request carrying the stand-in to any other place is stopped before anything of it is sent, and Activity names the key: "Claude Code tried to send your Anthropic key to …". A place for a key is one exact name, never "everything under example.com".
- **Only where a credential belongs.** See below: a stand-in anywhere else in a request is refused even on its way to the right place.
- **The real key never enters the agent's space.** It is not in its environment, its files or its command line, and the file Homebody keeps it in cannot be read from inside the fence.

## Using it

On an agent's page, under **Keys**, choose whose key it is, paste it, and check the place it is for. Homebody knows the usual place for the common keys (Anthropic, OpenAI, Google Gemini, OpenRouter, Mistral, Groq, DeepSeek, xAI, Hugging Face, GitHub); for any other, give the name the program reads it from and the place it may go.

From the command line (the key is read from input, never from the command line itself, where other users and your shell history would see it):

```bash
homebody key claude-code set ANTHROPIC_API_KEY
```

```bash
homebody key claude-code
```

```bash
homebody key claude-code remove ANTHROPIC_API_KEY
```

A key you add or remove reaches the agent the next time you open it. Replacing a key with a new one of the same kind keeps the stand-in, so an open session carries on.

## What it needs

**Look inside has to be on for that app**, and adding a key switches it on. The swap happens inside the encrypted connection, so Homebody has to be the one holding that connection. That is also why it is not offered for Ollama or for windowed apps, which cannot be looked inside.

## Where a key may sit in a request

The real key is put in only where the stand-in is the whole credential:

- the whole value of an `x-api-key`, `x-goog-api-key` or `api-key` header;
- `Authorization: Bearer <key>` or `Authorization: token <key>`;
- the name or the password of an `Authorization: Basic` header (which is how git sends a token);
- a parameter in the address that is exactly `key=<key>` (Google's API).

A request that carries the stand-in in any other part of its start is refused whole, even to the right place: in the path of the address, as another parameter, in any other header, glued to other text, or with the `TRACE` method. The reason is that servers repeat those parts back. An error page quotes the address it could not find; a redirect repeats the parameters; some headers come back in the answer. If the real key were put there, the agent would read it in the reply. Refusing is the only safe answer, and Activity says which key and why.

In the content of a request nothing is replaced. A stand-in that an agent repeats in what it sends is just a stand-in, and the check for secrets knows not to mistake it for a leak.

## What it does not do

- **It does not cover a sign-in.** If you signed an agent in with your account (Claude Code's `/login`, Codex with a ChatGPT plan), the agent holds that sign-in in its own private space. Homebody still decides where it can be sent, and the check for secrets stops it going out in the content of a request, but the agent has it.
- **The key is in a file, not in the Keychain.** `keys.json` in Homebody's own folder, readable only by your account. Any program running as you can read it, as it can read your shell's startup files, where keys usually live. What this protects against is the agent, not other software running as you.
- **A place that repeats requests back gives the key away.** If you name a place for a key and that place returns the headers it was sent, the agent can read the real key in the answer. Name the provider's own address, not a relay or a testing service.
- **In the Box it works the same way** ([box.md](box.md)): the Box holds a stand-in, and the real key never enters it.
- **A stand-in headed for the wrong place is caught in the start of a request only.** In the content of a request, in a web socket message, on a connection that is not looked inside, or disguised (escaped, split up), it goes out as what it is: a stand-in, which opens nothing. The real key is never involved. So "tried to send your key to …" in Activity is something you will see for the obvious attempts, not a promise that every attempt is announced.
- **A provider that repeats a credential header back would give the key away.** None of the built-in places is known to. Homebody does not read answers.
- **Not in a pipeline yet.** `homebody ci --env NAME` hands the real variable over, on purpose. A kept key of the same name takes precedence and the pipeline's is refused.

## How it was checked

Unit tests cover the swap byte for byte, however the request arrives (one byte at a time, or all at once), in each place a credential belongs, and that nothing of a request leaves when its stand-in is bound for the wrong place or sits in the wrong part. The bypass corpus (`app/tests/bypass-corpus.sh`, section "Inside: keys that stay home") runs it against the real engine: the app holds only a stand-in, cannot read the key file, is stopped four ways when it sends the stand-in somewhere else, and five ways when it tries to have the real key repeated back to it. End to end, with a made-up key and a public service that repeats requests back: the service received the real value, the app held only the stand-in.

The first version of this put the real key in wherever the stand-in appeared in the start of a request. An independent review, before anything was released, pointed out that an agent could then put its stand-in in the address and read the real key in the provider's own "no such address" reply. The rule above is the fix.
