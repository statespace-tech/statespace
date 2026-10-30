# ECS Fargate

Run the arms of the shop's experiments on ECS Fargate. `ssp proxy fargate`
runs as its own ECS service in front of the service `shop`. Each arm is an
ECS service in the same cluster. The service `shop` does not change.

| File | Purpose |
| --- | --- |
| `proxy-policy.json` | The permissions of the proxy's task role |
| `proxy-task-definition.json` | The proxy's task definition |
| `proxy-service.json` | The proxy's service, behind a load balancer |

Replace the account `123456789012`, the subnets, the security group, the
roles, and the load balancer with yours.

## Set up

1. Make the task role `statespace-proxy` with the policy
   `proxy-policy.json`. In the policy, use the task role and the execution
   role of `shop`, and the log group of `shop`.
2. Store a runtime token in Secrets Manager:

   ```shell
   ssp token add --app shop-prod --note "Fargate proxy" --json \
     | jq -r .token \
     | aws secretsmanager create-secret --name statespace-token \
         --secret-string file:///dev/stdin
   ```

   The execution role of the proxy must read this secret.
3. In the security group of `shop`, let the security group of the proxy
   connect to the app's port. The arms use the security group of `shop`.
4. Register the task definition and make the service:

   ```shell
   aws ecs register-task-definition \
     --cli-input-json file://proxy-task-definition.json
   aws ecs create-service --cli-input-json file://proxy-service.json
   ```

5. Point the public load balancer or the DNS name at the proxy's target
   group instead of the target group of `shop`.
6. Check that the proxy reports, and that the app has a live commit:

   ```shell
   ssp app show shop-prod
   ```

## What the proxy does

- Live is the image of the task definition of the primary deployment of
  `shop`. When that image has a tag, the proxy reports the digest that the
  running tasks of `shop` use.
- `--live-url` is where live traffic goes: the internal load balancer of
  `shop`, or its Service Connect name. Without `--live-url`, the proxy uses
  the first Service Connect name of `shop`.
- For each arm of a starting or running experiment, the proxy copies the
  task definition of `shop` and changes only the image. Then it makes the
  service `shop-ssp-<experiment>-<commit 12>`, such as
  `shop-ssp-checkout-a1b2c3d4e5f6`. A name of more than 255 characters is
  cut and ends with a short hash.
- The service of an arm copies the subnets, the security groups, the public
  IP setting, the launch type or capacity providers, the platform version,
  and the deployment settings of `shop`. It does not copy the load balancers
  or the service discovery of `shop`. Thus an arm gets no live traffic.
- The task definition and the service of an arm have the tags
  `statespace.com/app`, `statespace.com/experiment`, and
  `statespace.com/commit`, and the tags of `shop`.
- The desired count of an arm is the desired count of `shop` times the
  share, at least 1.
- An arm is ready when one of its tasks runs the arm's image and is healthy.
  A container without a health check must accept a TCP connection.
- The proxy sends each request to the private IP address of a task of the
  arm. Arms with more tasks get requests on a different task after each
  poll.
- The proxy reads the output of the arms from CloudWatch Logs. Live's
  container must log with `awslogs`.
- When an experiment stops, the proxy deletes the services of its arms, and
  deregisters and deletes their task definitions.

Deploy a new live as usual. The arms stay.

```shell
aws ecs update-service --cluster shop --service shop \
  --task-definition shop:43
```

## Credentials

The proxy uses the AWS default chain: environment variables, a profile, web
identity, the ECS task role, or the EC2 instance role. On Fargate, it uses
its task role.

When the proxy runs outside the VPC of the tasks, add `--unit-ip public`.
The proxy then sends to the public IP address of each task, and also needs
`ec2:DescribeNetworkInterfaces`. Use this only for tests.
