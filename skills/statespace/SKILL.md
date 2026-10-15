---
name: statespace
description: A/B test versions of an app in production with the Statespace `ssp` CLI. Use when asked to set up Statespace, test a code change on live traffic, compare a branch with production, roll out a change gradually, or read experiment results.
---

# Statespace

Statespace lets you A/B test multiple versions of an app in production.

- A **variant** is a signed image of the app. Usually, you build it from a branch.
- An **experiment** gives a share of the app's subjects to each variant. **Control** gets
  the same share. Control is a copy of the deployed image. Statespace pins it when the
  experiment starts.
- All other subjects get the **default**, which is the deployed app.
- `ssp proxy` sends each subject to its arm. Each arm records all lines that it prints.

Use the CLI to change things. Use SQL to read data. Add `--json` to each `ssp` command to
get one JSON document. With `--json`, an error is `{"error": "..."}` on standard error,
with exit status 1.

## Set up

1. Install the CLI if `ssp` is not available.

   ```shell
   curl -fsSL https://statespace.com/install.sh | bash
   ```

2. Check the sign-in. If `ssp whoami` fails, ask the user to run `ssp login`. This
   command opens a browser, so the user must run it. In CI and on servers, use a token in
   `SSP_TOKEN`.

   ```shell
   ssp whoami --json
   ```

3. Find the app. If there is no app, ask the user for a name and create it.

   ```shell
   ssp app list --json
   ssp app create <app> --json
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
   ssp experiment list --app <app> --json
   psql "$DATABASE_URL" -c "SELECT * FROM <app>.notes ORDER BY timestamp DESC LIMIT 20"
   psql "$DATABASE_URL" -c "SELECT data->>'event' AS event, count(*) FROM <app>.output
                            WHERE data IS NOT NULL GROUP BY 1 ORDER BY 2 DESC LIMIT 20"
   ```

2. Make one change on a branch. To measure a new outcome, print a JSON line where it
   occurs. Add the subject from the `x-statespace-subject` request header:
   `{"event": "<what occurred>", "subject": "<subject id>", ...fields}`.

3. Make sure that the image of the branch starts and answers requests.

   ```shell
   docker build -t <app>:mine . && docker run --rm -p 8000:8000 <app>:mine
   ```

4. Push the branch. CI builds and pushes its image. Then CI publishes the image with a
   signature. Only CI has the signing key.

   ```shell
   ssp variant create <variant> --app <app> --image <registry>/<app>:<commit> --json
   ```

5. Create the experiment with a note that gives the hypothesis. Start with a small split.
   `--split` gives the share of all subjects for each variant. Control gets the same
   share.

   ```shell
   ssp experiment create <experiment> --app <app> --split <variant>=0.02 \
     --note "<why this change can move the outcome>" --json
   ssp experiment start <experiment> --note "<what this run tests>" --json
   ```

6. Make sure that the arms run before you wait for outcomes.

   ```shell
   psql "$DATABASE_URL" -c "SELECT * FROM <app>.hosts"
   psql "$DATABASE_URL" -c "SELECT variant, event, data, timestamp FROM <app>.lifecycle
                            ORDER BY timestamp DESC LIMIT 20"
   ```

7. Wait for outcomes. Then compare the arms and make a decision: increase the split,
   publish a fix, or stop. Write a note with the evidence for each decision.

   ```shell
   psql "$DATABASE_URL" -c "SELECT * FROM <app>.sample_ratio WHERE experiment = '<experiment>'"
   psql "$DATABASE_URL" -c "SELECT variant, count(DISTINCT subject_id) AS subjects,
       count(DISTINCT subject_id) FILTER (WHERE data->>'event' = '<event>') AS converted
     FROM <app>.logs WHERE experiment = '<experiment>' GROUP BY variant"
   ssp experiment update <experiment> --split <variant>=0.1 --json
   ssp experiment start <experiment> --json
   ssp experiment stop <experiment> --note "<result and decision>" --json
   ```

## Rules

- Put one idea in each variant. Then a result shows its cause.
- Compare a variant with the control arm of its experiment. Do not compare it with the
  default. The variant and control are both new containers with the same share of
  subjects.
- Each arm uses the same database and services as the default. Do not change the database
  schema in a variant. Put migrations in the deployed code first.
- Compare the arms before you increase the split. A difference within noise is not
  evidence.
- If `sample_ratio` shows a mismatch, the subjects did not split as the shares tell. Do
  not trust the comparison. Find `variant-unavailable` runs, and read the `lifecycle` and
  `stderr` lines of the arm. Repair the arm before you read outcomes.
- A rate such as errors needs a denominator. Divide by the rows of the arm in `requests`.
- `ssp experiment start` fails when other experiments of the app use the subjects that it
  needs. It shows a warning when the proxies run fewer arms than it needs.
- To ship a variant, merge its branch and deploy it as usual. Statespace does not deploy
  the default.
- `ssp app delete`, `ssp experiment delete`, and `ssp token revoke --all` need `--yes`.
  You cannot undo them. Ask the user before you run them.
