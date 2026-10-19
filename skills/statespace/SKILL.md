---
name: statespace
description: A/B test versions of an app in production with Statespace, through pull requests. Use when asked to set up Statespace, test a code change on live traffic, compare a branch with production, roll out a change gradually, or read experiment results.
---

# Statespace

Statespace lets you A/B test multiple versions of an app in production. Experiments run
from pull requests:

- A branch named `exp/<experiment>/<arm>` is an **arm**. `exp/<experiment>` is an
  experiment with one arm.
- `statespace.toml` on the branch gives the arm's `share` of the app's subjects, and its
  `subject` and `eligibility` rules.
- The arm runs the latest commit while GitHub says that its pull request can merge.
  GitHub's branch protection decides who must approve.
- Each arm is compared with a **control**: the default that its branch started from. The
  control gets the share of the largest arm with the same base.
- All other subjects get the **default**, which is the deployed app.
- `ssp proxy` sends each subject to its arm. Each arm records all lines that it prints.

You cannot start an arm with a command. You push commits and open pull requests; people
or review agents approve them. Use SQL to read data. Add `--json` to each `ssp` command
to get one JSON document. With `--json`, an error is `{"error": "..."}` on standard
error, with exit status 1.

## Set up

1. Install the CLI if `ssp` is not available.

   ```shell
   curl -fsSL https://statespace.com/install.sh | bash
   ```

2. Check the sign-in. If `ssp whoami` fails, ask the user to run `ssp login`. This
   command opens a browser, so the user must run it.

   ```shell
   ssp whoami --json
   ```

3. If the repository has no `statespace.toml`, set it up. Ask the user for the app name.
   Then ask the user to install the GitHub App with `ssp repo connect`, to add the
   production step that `ssp init github` prints, and to require one approval in branch
   protection.

   ```shell
   ssp init github --app <app> --json
   ```

4. Get a database URL to read the data. The token secret shows only once. Do not write it
   to files that Git tracks.

   ```shell
   ssp token create --name agent-analyst --preset analyst --app <app> --json
   export DATABASE_URL='<database_url from the output>'
   ```

If the user did not give a task, stop after the setup. Then ask which change to test.

## The loop

1. Before you change code, read what other experiments tested and found.

   ```shell
   ssp status --json
   gh pr list --search "head:exp/" --state all --limit 20
   psql "$DATABASE_URL" -c "SELECT data->>'event' AS event, count(*) FROM <app>.output
                            WHERE data IS NOT NULL GROUP BY 1 ORDER BY 2 DESC LIMIT 20"
   ```

2. Start a branch from the latest default branch. Make one change. To measure a new
   outcome, print a JSON line where it occurs. Add the subject from the
   `x-statespace-subject` request header:
   `{"event": "<what occurred>", "subject": "<subject id>", ...fields}`.

   ```shell
   git switch -c exp/<experiment>/<arm> origin/main
   ```

3. Set the share in `statespace.toml`. Start small, such as `share = 0.02`. Check the
   file, and test the rules with a request.

   ```shell
   ssp check --json
   ssp check --path / --header cf-ipcountry=US --json
   ```

4. Make sure that the image starts and answers requests.

   ```shell
   docker build -t <app>:mine . && docker run --rm -p 8000:8000 <app>:mine
   ```

5. Push, and open a pull request. In the description, give the hypothesis: why the change
   can move the outcome. CI records each commit with `ssp apply`.

   ```shell
   git push -u origin exp/<experiment>/<arm>
   gh pr create --fill
   ```

6. Read the `statespace/arm` check. It tells whether the arm runs, and why it waits: for
   approval, for a check, for a deploy of its base, or for traffic. Do not try to go
   around a wait.

   ```shell
   gh pr checks
   ssp status <experiment> --json
   ```

7. Wait for outcomes. Then compare each arm with its control, and make a decision: push a
   fix, raise the share in `statespace.toml`, make the pull request a draft to stop, or
   ask to merge. Write the evidence in a pull request comment.

   ```shell
   psql "$DATABASE_URL" -c "SELECT * FROM <app>.sample_ratio WHERE experiment = '<experiment>'"
   psql "$DATABASE_URL" -c "SELECT arm, commit, count(DISTINCT subject_id) AS subjects,
       count(DISTINCT subject_id) FILTER (WHERE data->>'event' = '<event>') AS converted
     FROM <app>.logs WHERE experiment = '<experiment>' GROUP BY arm, commit"
   ```

## Rules

- Put one idea in each arm. Then a result shows its cause.
- Compare an arm with the control of its base. In `<app>.arms`, that control's `commit`
  is the arm's `base_commit`. Do not compare an arm with the default.
- Each new commit needs approval again before it runs, unless the repository says
  otherwise. Push only commits that you want users to see.
- Arms of one experiment must use the same `subject` and `eligibility`. To test another
  audience, use another experiment name.
- Merging an arm ends its experiment. The other arms run again only after you update
  them with the default branch.
- Each arm uses the same database and services as the default. Do not change the database
  schema in an arm. Put migrations in the deployed code first.
- Compare the arms before you raise the share. A difference within noise is not evidence.
- If `sample_ratio` shows a mismatch, the subjects did not split as the shares tell. Do
  not trust the comparison. Find `arm-unavailable` runs, and read the `lifecycle` and
  `stderr` lines of the arm. Repair the arm before you read outcomes.
- A rate such as errors needs a denominator. Divide by the rows of the arm in `requests`.
- `ssp stop` is for emergencies. It stops arms at once and makes their pull requests
  drafts. Ask the user before you run it, unless the arm harms users.
- `ssp app delete` and `ssp token revoke --all` need `--yes`. You cannot undo them. Ask
  the user before you run them.
