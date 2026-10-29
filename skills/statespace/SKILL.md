---
name: statespace
description: Run a commit of an app on a share of live traffic beside a control with Statespace, and read the evidence. Use when asked to set up Statespace, test a code change on live traffic, compare a commit with production, or read arm results.
---

# Statespace

Statespace runs a commit of an app on a small share of live traffic, beside a control,
and records what happens.

- An **app** is one running copy of a service, such as `shop-prod`. A proxy is in front
  of it.
- An **arm** is a commit. The commit is its ID. You add it with `ssp arm add`.
- The **control** is a copy of the image that live runs now. It gets as many subjects
  as the largest arm.
- **Live** gets everyone else. Statespace never records live.
- Settings are in `statespace.toml`. They change only by commits. The settings of the
  build that live runs apply.
- The evidence is on the arm's commit, as the commit status `statespace/<app>`, and in
  `ssp arm show`.

Statespace never starts an arm by itself. A person or an agent with permission on the
GitHub repository adds it. Add `--json` to each `ssp` command to get one JSON document.
With `--json`, an error is `{"error": "..."}` on standard error, with exit status 1. No
command asks a question, except `ssp login`.

## Credentials

The CLI uses the first credential that it finds:

1. `STATESPACE_TOKEN`.
2. The session of `ssp login`.
3. A GitHub user token in `GH_TOKEN` or `GITHUB_TOKEN` (`gho_`, `ghp_`, `ghu_`,
   `github_pat_`).
4. In GitHub Actions, the job's OIDC token. The job needs `permissions: id-token: write`.

Check the credential first:

```shell
ssp whoami --json
```

If it fails and you have no GitHub token, ask the user to run `ssp login`. This command
opens a browser, so the user must run it.

## Set up

1. Install the CLI if `ssp` is not available.

   ```shell
   curl -fsSL https://statespace.com/install.sh | bash
   ```

2. Ask the user to install the Statespace GitHub App on the repository from
   https://statespace.com.

3. In the app's folder, write `statespace.toml` and get the CI step. The command fails
   when the file exists. It changes only the working copy.

   ```shell
   ssp init --json
   ```

4. Add the step after the image build, in the workflow that builds every commit that can
   become an arm. Give the image by digest. Commit the workflow and `statespace.toml`.

   ```yaml
   - uses: statespace-tech/action@v1
     with:
       image: <registry>/<image>@${{ steps.build.outputs.digest }}
       path: <folder of statespace.toml>
   ```

5. Add an app for each running copy. In the app's folder, the command reads the
   repository and the folder. Read what it detected.

   ```shell
   ssp app add <app> --note "<what this copy is>" --json
   ```

6. Ask the user to add a runtime token and to run `ssp proxy` in front of the app. The
   token shows only once. Do not write it to files that Git tracks.

   ```shell
   ssp token add --app <app> --note "<where the proxy runs>" --json
   ```

7. Check that a proxy reports and that the app has a live commit:

   ```shell
   ssp app show <app> --json
   ```

## Test a change

1. Make the change in a commit. Push it, so that CI builds it and records the build.
2. Add the commit as an arm. The command prints the build that it chose.

   ```shell
   ssp arm add <commit> --app <app> --share 0.05 --note "<hypothesis>" --json
   ```

   If it fails with "No build of <commit>", wait for CI, then try again.
3. Read the arm's state, the reason when it waits, and the evidence:

   ```shell
   ssp arm show <commit> --app <app> --json
   ```

   - `state` is `starting`, `running`, `waiting`, `stopped`, or `removed`.
   - `totals` has `subjects`, `requests`, `errors`, `p50_ms`, and `p90_ms` for the arm
     and the control. `metrics` has each `[[metric]]` for both sides.
   - `split` tells whether the subjects match the shares. Do not trust evidence when
     `split.ok` is false.
   - `ahead` and `behind` give the distance from the live commit.
4. Decide with the user. Statespace gives evidence, not verdicts.
5. End the arm. Statespace keeps its evidence.

   ```shell
   ssp arm remove <commit> --app <app> --note "<decision>" --json
   ```

## Stop in an emergency

A stop places a hold. The arm leaves traffic in seconds.

```shell
ssp arm stop <commit> --app <app> --note "<why>" --json
ssp app stop <app> --note "<why>" --json
```

`ssp arm resume` and `ssp app resume` remove exactly the hold that they name.

## Events and metrics

Add events and metrics to `statespace.toml` in a commit. They take effect when that
commit's build runs as live.

```toml
[[event]]
name = "signup"
match = 'request.method == "POST" && request.path == "/signup" && request.status < 300'

[[event]]
name = "fatal"
match = 'stderr.text.contains("FATAL")'

[[metric]]
label = "signup rate %"
value = '100.0 * events["signup"].subjects / subjects'
```

- Each `match` uses one source: `request`, `stdout`, or `stderr`.
- `request` has `method`, `path`, `query`, `headers`, `cookies`, `status`, and `ms`.
- `stdout` and `stderr` have `text` and `json` (the line as a JSON object, or `{}`).
- A missing field is no match.
- Output events have `.lines` only. Request events have `.lines` and `.subjects`.

## Rules

- Do not add an arm without the user's consent.
- Use `--note` on each command that changes something.
- Never put a token in a file that Git tracks.
- Read the reason when an arm waits. Do not add arms to work around `max_arms` or
  `max_traffic`.
