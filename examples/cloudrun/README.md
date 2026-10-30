# Cloud Run

Run the arms of the shop's experiments on Cloud Run. `ssp proxy cloudrun` runs as its
own Cloud Run service in front of the service `shop`. Each arm is a revision of `shop`
with 0% traffic and a tag.

## Set up

1. Make a service account for the proxy, and give it what it needs on `shop`:

   ```shell
   gcloud iam service-accounts create statespace-proxy
   proxy=statespace-proxy@acme.iam.gserviceaccount.com
   # Make revisions of shop and change its traffic tags.
   gcloud run services add-iam-policy-binding shop --region us-east1 \
     --member "serviceAccount:$proxy" --role roles/run.developer
   # Send requests to shop and its tags.
   gcloud run services add-iam-policy-binding shop --region us-east1 \
     --member "serviceAccount:$proxy" --role roles/run.invoker
   # Deploy revisions that run as shop's own service account.
   gcloud iam service-accounts add-iam-policy-binding \
     shop@acme.iam.gserviceaccount.com \
     --member "serviceAccount:$proxy" --role roles/iam.serviceAccountUser
   # Deploy the images of the arms. Cloud Run checks that the caller can read them.
   gcloud artifacts repositories add-iam-policy-binding shop --location us-east1 \
     --member "serviceAccount:$proxy" --role roles/artifactregistry.reader
   # Read the output of the arms.
   gcloud projects add-iam-policy-binding acme \
     --member "serviceAccount:$proxy" --role roles/logging.viewer
   ```

2. Store a runtime token in Secret Manager:

   ```shell
   ssp token add --app shop-prod --note "Cloud Run proxy" --json \
     | jq -r .token \
     | gcloud secrets create statespace-token --data-file=-
   ```

3. Deploy the proxy:

   ```shell
   gcloud run deploy statespace-proxy --region us-east1 \
     --image ghcr.io/statespace-tech/statespace:0.4.0 \
     --service-account "$proxy" \
     --set-secrets STATESPACE_TOKEN=statespace-token:latest \
     --set-env-vars STATESPACE_APP=shop-prod \
     --no-cpu-throttling --min-instances 1 \
     --args proxy,cloudrun,--service,shop,--region,us-east1
   ```

   The proxy works between requests: it renews its lease, starts and stops arms, and
   sends records. Thus it needs CPU that is always allocated, and one instance at
   minimum.

4. Point the load balancer or the domain mapping at `statespace-proxy` instead of
   `shop`.

## What the proxy does

- Live is the image of the revision of `shop` that gets the most traffic.
- For each arm of a starting or running experiment, the proxy copies the template of
  `shop`, changes only the image, and makes a revision with 0% traffic and the tag
  `ssp-<experiment>-<commit 12>`, such as `ssp-checkout-a1b2c3d4e5f6`.
- The tag and the service name together have at most 46 characters. A longer tag
  is cut and ends with a short hash. The labels `statespace-experiment` and
  `statespace-commit` of the revision keep the full values.
- While the proxy makes the revision, it pins the traffic of `shop` to the revision
  that has it now, so the new revision gets none. When the revision exists, the
  proxy gives `shop` its own template and traffic again, and adds only the tag.
- The proxy sends each request to the tag URL, with an ID token for the URL of
  `shop` in the header `X-Serverless-Authorization`. The app keeps its own
  `Authorization` header.
- The proxy reads the output of the arms from Cloud Logging.
- When an experiment stops, the proxy removes the tags of its arms and deletes their
  revisions.

Deploy a new live as usual. The new revision gets all the traffic, and the tags of
the arms stay.

```shell
gcloud run deploy shop --region us-east1 --image ghcr.io/acme/shop@sha256:3333…
```

The template of `shop` keeps the name of live's revision. A tool that sends the
template back with a new image must clear `template.revision`, as `gcloud run deploy`
does.

If an earlier version of the proxy pinned the traffic of `shop`, send the traffic to
the latest revision one time:

```shell
gcloud run services update-traffic shop --region us-east1 --to-latest
```

The arms are fixed commits. A new live does not change them.
