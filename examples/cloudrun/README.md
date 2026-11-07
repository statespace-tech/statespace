# Google Cloud Run

Run the arms of the shop's experiments on Cloud Run. `ssp proxy cloudrun` runs as its
own Cloud Run service in front of the service `shop`. Each arm is its own Cloud Run
service, a copy of `shop` with another image. The proxy never changes `shop`.

## Set up

1. Make a service account for the proxy, and a custom role with only what the proxy
   needs:

   ```shell
   gcloud iam service-accounts create statespace-proxy
   proxy=statespace-proxy@acme.iam.gserviceaccount.com
   gcloud iam roles create statespaceProxy --project acme \
     --title "Statespace proxy" \
     --permissions run.services.create,run.services.delete,run.services.get,run.services.list,run.services.getIamPolicy,run.services.setIamPolicy,run.revisions.get,run.operations.get,run.routes.invoke
   ```

2. Give the service account what it needs:

   ```shell
   # Make, read, and delete the services of the arms, and call shop and the arms.
   gcloud projects add-iam-policy-binding acme \
     --member "serviceAccount:$proxy" --role projects/acme/roles/statespaceProxy
   # Deploy arms that run as shop's own service account.
   gcloud iam service-accounts add-iam-policy-binding \
     shop@acme.iam.gserviceaccount.com \
     --member "serviceAccount:$proxy" --role roles/iam.serviceAccountUser
   # Deploy the images of the arms. Cloud Run checks that the caller can read them.
   gcloud artifacts repositories add-iam-policy-binding shop --location us-east1 \
     --member "serviceAccount:$proxy" --role roles/artifactregistry.reader
   ```

3. Store a runtime token in Secret Manager. The proxy asks Statespace which app the
   token is for.

   ```shell
   ssp token add --app shop-prod --note "Cloud Run proxy" --json \
     | jq -r .token \
     | gcloud secrets create statespace-token --data-file=-
   ```

4. Deploy the proxy:

   ```shell
   gcloud run deploy statespace-proxy --region us-east1 \
     --image ghcr.io/statespace-tech/statespace:0.5.1 \
     --service-account "$proxy" \
     --set-secrets STATESPACE_TOKEN=statespace-token:latest \
     --no-cpu-throttling --min-instances 1 \
     --args proxy,cloudrun,--service,shop,--region,us-east1
   ```

   The proxy works between requests: it renews its lease, starts and stops arms, and
   sends counts. Thus it needs CPU that is always allocated, and one instance at
   minimum.

5. Point the load balancer or the domain mapping at `statespace-proxy` instead of
   `shop`. `shop` keeps its own URL, so a load balancer can send traffic to `shop`
   when the proxy is down.

## What the proxy does

- Live is the image of the revision of `shop` that gets the most traffic.
- For each arm of a starting or running experiment, the proxy makes a service
  `shop-ssp-<experiment>-<commit 12>`, such as `shop-ssp-checkout-a1b2c3d4e5f6`, in
  the same project and region. A name longer than 49 characters is cut and ends with
  a short hash. The labels `statespace-app`, `statespace-experiment`, and
  `statespace-commit` keep the full values.
- The arm's service is a copy of the template of `shop` with only the image changed:
  containers, environment, resources, service account, VPC access, scaling, timeout,
  and execution environment. It has the ingress, launch stage, Binary Authorization
  policy, and labels of `shop`. It gets no traffic split, custom domain, or IAM
  policy of `shop`. No variable that starts with `STATESPACE_` is copied.
- Only the proxy's service account may call an arm's service: the proxy gives itself
  Cloud Run Invoker on it.
- The proxy sends each request to the arm's URL, with an ID token for that URL in
  the header `X-Serverless-Authorization`. The app keeps its own `Authorization`
  header.
- When an experiment stops, the proxy deletes the services of its arms.

Deploy a new live as usual, with `gcloud run deploy`, Terraform, or any other tool.
The proxy changes nothing in `shop`, so your tool sees no difference. The arms are
fixed commits. A new live does not change them.
