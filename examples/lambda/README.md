# AWS Lambda

Run the arms of the shop's experiments on AWS Lambda. `ssp proxy lambda`
runs as its own ECS service in front of the function `shop`. Each arm is a
Lambda function in the same account and region. The function `shop` does not
change.

| File | Purpose |
| --- | --- |
| `Dockerfile` | The tiny app of `examples/app` as a function, with the AWS Lambda Web Adapter |
| `proxy-policy.json` | The permissions of the proxy's task role |
| `proxy-task-definition.json` | The proxy's task definition |
| `proxy-service.json` | The proxy's service, behind a load balancer |

Replace the account `123456789012`, the subnets, the security group, the
roles, and the load balancer with yours.

## Set up

1. Deploy `shop` from a container image in Amazon ECR, and give the version
   that runs live the alias `live`. CI records the image of each commit, as
   on the other runtimes.
2. Make the task role `statespace-proxy` with the policy
   `proxy-policy.json`. In the policy, use the execution role of `shop`.
3. Store a runtime token in Secrets Manager:

   ```shell
   ssp token add --app shop-prod --note "Lambda proxy" --json \
     | jq -r .token \
     | aws secretsmanager create-secret --name statespace-token \
         --secret-string file:///dev/stdin
   ```

   The execution role of the proxy must read this secret.
4. Register the task definition and make the service:

   ```shell
   aws ecs register-task-definition \
     --cli-input-json file://proxy-task-definition.json
   aws ecs create-service --cli-input-json file://proxy-service.json
   ```

5. Point the public load balancer or the DNS name at the proxy's target
   group instead of the API Gateway or function URL of `shop`.
6. Check that the proxy reports, and that the app has a live commit:

   ```shell
   ssp app show shop-prod
   ```

## What the proxy does

- Live is the image digest of `shop` at `--qualifier`, or at `$LATEST`.
- The proxy invokes live and each arm with the event of a function URL
  (payload format 2.0), and turns the answer into the response, as a
  function URL does. The app sees the same event as behind a function URL
  or an API Gateway HTTP API. A web framework with the AWS Lambda Web
  Adapter, such as this example, takes it as an HTTP request.
- With `--live-url`, live traffic goes to that HTTP address instead, such as
  the function URL of `shop`. The arms are still invoked.
- For each arm of a starting or running experiment, the proxy makes the
  function `shop-ssp-<experiment>-<commit 12>`, such as
  `shop-ssp-checkout-a1b2c3d4e5f6`, with the configuration of `shop` and the
  arm's image. A name of more than 64 characters is cut and ends with a
  short hash.
- The function of an arm copies the role, memory, timeout, architecture,
  environment, VPC, file systems, tracing, image settings, and log format of
  `shop`. It gets no trigger and no function URL. Thus an arm gets no live
  traffic. It logs to its own log group, `/aws/lambda/<name>`.
- The function of an arm has the tags `statespace.com/app`,
  `statespace.com/experiment`, and `statespace.com/commit`, and the tags of
  `shop`.
- An arm is ready when Lambda says that its function is active.
- A request body can be at most 6 MB, the limit of an invocation.
- When an experiment stops, the proxy deletes the functions of its arms. The
  log groups of the arms stay.

Deploy a new live as usual. The arms stay.

```shell
aws lambda update-function-code --function-name shop \
  --image-uri 123456789012.dkr.ecr.us-east-1.amazonaws.com/shop@sha256:<digest> \
  --publish
aws lambda update-alias --function-name shop --name live \
  --function-version <version>
```

## Try the app

From the root folder of the repo, build the app of this folder, and push it
to Amazon ECR. Lambda reads only images with one platform, so turn off the
provenance attestation.

```shell
docker build --provenance=false --platform linux/amd64 \
  --build-arg GREETING=hi -f examples/lambda/Dockerfile \
  -t 123456789012.dkr.ecr.us-east-1.amazonaws.com/shop:hi examples
docker push 123456789012.dkr.ecr.us-east-1.amazonaws.com/shop:hi
```

## Credentials

The proxy uses the AWS default chain: environment variables, a profile, web
identity, the ECS task role, or the EC2 instance role. On Fargate, it uses
its task role.
