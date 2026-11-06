# The tiny app of examples/app as a Lambda function. The AWS Lambda Web
# Adapter turns each invocation into an HTTP request to the app. Build it
# from the root folder of the repo:
#   docker build --provenance=false -f examples/lambda/Dockerfile -t shop:hello examples
FROM python:3.12-slim
COPY --from=public.ecr.aws/awsguru/aws-lambda-adapter:0.9.1 /lambda-adapter /opt/extensions/lambda-adapter
ARG GREETING=hello
ENV GREETING=$GREETING PORT=8000
COPY app/app.py /app/app.py
CMD ["python3", "/app/app.py"]
