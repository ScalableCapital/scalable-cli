# Configuration

Storage backends, local controls, and dedicated-user setups for Scalable CLI.

[Scalable CLI](https://github.com/ScalableCapital/scalable-cli) · [Command reference](commands.md) · [Automation](automation.md)

## Configuration file

`config.toml` is optional local runtime configuration. Defaults are
platform-specific.

macOS example:

```toml
[auth]
session_backend = "keyring"
signing_key_backend = "secure_enclave"
```

Configuration options:

- session_backend: where the login session is stored. Supported values: `keyring`, `file`. Default is `keyring` on macOS/Linux.
- signing_key_backend: where the authentication signing key is stored. Supported values: `file`, `secure_enclave`, `pkcs11`. Default is `secure_enclave` on macOS and `file` on Linux. `secure_enclave` is macOS-only. `pkcs11` is Linux-only and opt-in.

Config file locations:

- macOS: `~/.config/scalable-cli/config.toml`
- Linux: `$XDG_CONFIG_HOME/scalable-cli/config.toml` when `XDG_CONFIG_HOME` is
  set, otherwise `~/.config/scalable-cli/config.toml`

## Linux servers and containers

The default session backend on Linux uses the system keyring through Secret
Service. Servers and containers without this service may need file-based
session storage.

Add the following to `config.toml` at the location listed above:

```toml
[auth]
session_backend = "file"
```

Then authenticate and verify the session:

```bash
sc login
sc whoami
```

Complete login yourself in a browser. File-based storage keeps session secrets
in local files rather than the system keyring; keep those files private to the
Unix account running the CLI. Prefer keyring-backed storage where available.

## Local read-only mode

- `sc login --local-read-only` stores the session in a locally enforced read-only mode.
- This does not change token permissions or backend access.
- Read commands and local commands such as `sc broker context select` continue to
  work, while write commands are blocked locally until you run `sc login` again
  without `--local-read-only`.

## Local trade controls

You can optionally define local trade controls for the CLI:

```toml
[trade_controls]
allowed_isins = ["US0378331005", "IE00B4L5Y983"]
denied_isins = ["US88160R1014"]
max_order_notional = "1000"
```

Trade-control behavior:

- If a control is not configured, it is inactive.
- If neither `allowed_isins` nor `denied_isins` is configured, ISIN controls are inactive.
- If `allowed_isins = []`, no ISIN can be traded through the CLI.
- If both `allowed_isins` and `denied_isins` are configured, the effective set is `allowed_isins - denied_isins`.
- `max_order_notional` is checked against the prepared order notional before submission.

`max_order_notional` checks the estimated order volume, rounded to four decimal
places; it is not a maximum execution-cost guarantee. For a buy limit above the
stop, execution cost can exceed the estimate.

These controls apply only when trading through `sc`. They do not change account
permissions or backend trading permissions.

## Linux PKCS#11 signing key

PKCS#11 lets the CLI use an existing hardware token, smartcard, or HSM-backed
key for its authentication signing key. The CLI does not create, import, rotate,
delete, export, or store PKCS#11 private keys.

```toml
[auth]
signing_key_backend = "pkcs11"

[auth.pkcs11]
module_path = "/usr/lib/x86_64-linux-gnu/opensc-pkcs11.so"
key_uri = "pkcs11:token=YubiKey%20PIV;id=%01"
```

PKCS#11 options:

- module_path: required path to the PKCS#11 provider library.
- key_uri: required PKCS#11 URI for the existing key.

The URI must start with `pkcs11:` and identify exactly one EC P-256 private
key. Use `id=` or `object=` to identify the key; add `token=`, `serial=`,
`manufacturer=`, or `model=` if needed to make token selection unambiguous.
Percent-encode binary IDs and spaces.

If the token requires login, use either the token/provider protected
authentication path or set `SC_PKCS11_PIN` in the environment. The CLI does not
prompt for or store the PIN.

PKCS#11 failures do not fall back to `file`. Switching signing backends changes
the authentication key identity, so run `sc login` again after changing this
setting.

## Run as a dedicated Unix user

If another local service needs to invoke `sc`, it can be useful to run the CLI as
a separate Unix user with a narrowly scoped `sudoers` rule.

This lets the application use the CLI without gaining direct access to files
created by the CLI, such as session or configuration data owned by
`scalable-cli-user`. That separation is useful for limiting data exposure and
reducing the blast radius if the calling application account is compromised.

### Create the user

Create the dedicated account:

```bash
sudo useradd -m -s /bin/bash scalable-cli-user
```

If the user should not be able to log in interactively, use a non-login shell
instead:

```bash
sudo useradd -m -s /usr/sbin/nologin scalable-cli-user
```

Verify that the user exists:

```bash
id scalable-cli-user
```

Run the initial `sc login` as this Unix user so it owns the session. Complete
login yourself in a browser. The usual [session limits](automation.md#session-lifetimes-and-refresh)
also apply to dedicated-user setups.

### Configure `sudoers`

Assume the main application runs as Unix user `app-user` and should be allowed
to execute exactly the `sc` CLI as `scalable-cli-user`.

Create a dedicated `sudoers` file such as `/etc/sudoers.d/app-cli` with:

```sudoers
app-user ALL=(scalable-cli-user) NOPASSWD: /usr/local/bin/sc
```

This means:

- `app-user` may use `sudo` without a password.
- The command runs as `scalable-cli-user`.
- Only `/usr/local/bin/sc` is allowed.

Set the required permissions on the file:

```bash
sudo chmod 440 /etc/sudoers.d/app-cli
```

Validate the `sudoers` configuration:

```bash
sudo visudo -cf /etc/sudoers.d/app-cli
```

### Result

With that setup, `app-user` can run:

```bash
sudo -n -u scalable-cli-user /usr/local/bin/sc
```

The same rule does not allow arbitrary commands to run as
`scalable-cli-user`.

In practice, this means `app-user` can trigger `sc`, but does not automatically
get read access to files owned by `scalable-cli-user`. The CLI remains usable
while its local files stay isolated behind normal Unix file ownership.
