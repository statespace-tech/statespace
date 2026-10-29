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
     --image ghcr.io/statespace-tech/statespace:0.3.0 \
     --service-account "$proxy" \
     --set-secrets STATESPACE_TOKEN=statespace-token:latest \
     --set-env-vars STATESPACE_APP=shop-prod \
     --args proxy,cloudrun,--service,shop,--region,us-east1
   ```

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
- The first time, the proxy pins the traffic of `shop` to the revision that has it
  now, so a new revision gets none.
- The proxy sends each request to the tag URL, with an ID token in the header
  `X-Serverless-Authorization`. The app keeps its own `Authorization` header.
- The proxy reads the output of the arms from Cloud Logging.
- When an experiment stops, the proxy removes the tags of its arms and deletes their
  revisions.

Because the traffic of `shop` is pinned, a deploy does not move traffic by itself.
Deploy a new live, then send the traffic to it. The tags of the arms stay.

```shell
gcloud run deploy shop --region us-east1 --image ghcr.io/acme/shop@sha256:3333…
gcloud run services update-traffic shop --region us-east1 --to-latest
```

The arms are fixed commits. A new live does not change them.
