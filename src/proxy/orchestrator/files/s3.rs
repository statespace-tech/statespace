//! `ssp proxy s3`: build folders in Amazon S3, or in any S3-compatible store with
//! `--endpoint` (R2, B2, Spaces, MinIO). The proxy uses the AWS default
//! credential chain and SigV4, and needs `s3:GetObject` and
//! `s3:ListBucket` only.

use super::{FileRuntime, Listed, Object, Storage};
use crate::{
    cli::S3Args,
    proxy::orchestrator::aws::{Aws, Endpoints, Reply, Service, encode, xml_value},
};

/// `ssp proxy s3`.
pub type S3 = FileRuntime<S3Storage>;

pub struct S3Storage {
    aws: Aws,
}

impl S3 {
    pub async fn connect(arguments: S3Args) -> anyhow::Result<Self> {
        // An S3-compatible store may need no region, but SigV4 names one.
        let fallback = arguments.endpoint.is_some().then_some("us-east-1");
        let aws = Aws::connect(
            arguments.region.clone(),
            fallback,
            Endpoints {
                s3: arguments.endpoint.clone(),
                ..Endpoints::default()
            },
        )
        .await?;
        Ok(FileRuntime::new(
            S3Storage { aws },
            ("s3", "s3", "folders in S3"),
            arguments.bucket.clone(),
            &arguments.files,
        ))
    }
}

impl Storage for S3Storage {
    async fn list(&self, bucket: &str, prefix: &str) -> anyhow::Result<Vec<Listed>> {
        let mut listed = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut url = format!(
                "{}?encoding-type=url&list-type=2&prefix={}",
                self.aws.s3_url(bucket, ""),
                encode(prefix)
            );
            if let Some(token) = &token {
                url.push_str(&format!("&continuation-token={}", encode(token)));
            }
            let reply = self
                .aws
                .send(Service::S3, "GET", &url, &[], Vec::new())
                .await?
                .ok(&format!("the listing of s3://{bucket}/{prefix}"))?;
            let text = reply.text();
            listed.extend(contents(&text));
            token = match xml_value(&text, "IsTruncated").as_deref() {
                Some("true") => xml_value(&text, "NextContinuationToken"),
                _ => None,
            };
            if token.is_none() {
                return Ok(listed);
            }
        }
    }

    async fn get(&self, bucket: &str, key: &str) -> anyhow::Result<Option<Object>> {
        let url = self.aws.s3_url(bucket, key);
        let reply = self
            .aws
            .send(Service::S3, "GET", &url, &[], Vec::new())
            .await?;
        if reply.status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let reply = reply.ok(&format!("s3://{bucket}/{key}"))?;
        Ok(Some(object(reply)))
    }
}

fn object(reply: Reply) -> Object {
    Object {
        content_type: reply.header("content-type"),
        etag: reply.header("etag"),
        last_modified: reply.header("last-modified"),
        cache_control: reply.header("cache-control"),
        body: reply.body,
    }
}

/// The objects of a ListObjectsV2 answer with `encoding-type=url`.
fn contents(xml: &str) -> Vec<Listed> {
    xml.split("<Contents>")
        .skip(1)
        .filter_map(|block| {
            let block = &block[..block.find("</Contents>")?];
            let key = url_decode(&xml_value(block, "Key")?)?;
            let version = xml_value(block, "ETag").unwrap_or_default();
            Some(Listed { key, version })
        })
        .collect()
}

/// A key that Amazon S3 sent URL-encoded: `%XX`, and `+` for a space.
fn url_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' => {
                let pair = std::str::from_utf8(bytes.get(index + 1..index + 3)?).ok()?;
                out.push(u8::from_str_radix(pair, 16).ok()?);
                index += 3;
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_listing() {
        let xml = "<ListBucketResult><IsTruncated>false</IsTruncated>\
                   <Contents><Key>builds/a1/index.html</Key>\
                   <ETag>&quot;e1&quot;</ETag><Size>3</Size></Contents>\
                   <Contents><Key>builds/a1/a%20b+c.txt</Key><ETag>&quot;e2&quot;</ETag>\
                   </Contents></ListBucketResult>";
        assert_eq!(
            contents(xml),
            [
                Listed {
                    key: "builds/a1/index.html".into(),
                    version: "&quot;e1&quot;".into()
                },
                Listed {
                    key: "builds/a1/a b c.txt".into(),
                    version: "&quot;e2&quot;".into()
                },
            ]
        );
    }
}
