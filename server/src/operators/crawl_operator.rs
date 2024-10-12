use crate::data::models::CrawlOptions;
use crate::data::models::CrawlStatus;
use crate::data::models::FirecrawlCrawlRequest;
use crate::data::models::RedisPool;
use crate::handlers::chunk_handler::CrawlInterval;
use crate::{
    data::models::{CrawlRequest, CrawlRequestPG, Pool, ScrapeOptions},
    errors::ServiceError,
};
use actix_web::web;
use diesel::prelude::*;
use diesel::QueryDsl;
use diesel_async::RunQueryDsl;
use regex::Regex;
use reqwest::Url;
use scraper::Html;
use scraper::Selector;
use serde::{Deserialize, Serialize};

use super::parse_operator::convert_html_to_text;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct IngestResult {
    pub status: Status,
    pub completed: u32,
    pub total: u32,
    #[serde(rename = "creditsUsed")]
    pub credits_used: u32,
    #[serde(rename = "expiresAt")]
    pub expires_at: String,
    pub next: Option<String>,
    pub data: Option<Vec<Option<Document>>>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Scraping,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Document {
    pub markdown: Option<String>,
    pub extract: Option<String>,
    pub html: Option<String>,
    #[serde(rename = "rawHtml")]
    pub raw_html: Option<String>,
    pub links: Option<Vec<String>>,
    pub screenshot: Option<String>,
    pub metadata: Metadata,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Metadata {
    pub title: Option<String>,
    pub description: Option<String>,
    pub language: Option<String>,
    pub keywords: Option<String>,
    pub robots: Option<String>,
    #[serde(rename = "ogTitle")]
    pub og_title: Option<String>,
    #[serde(rename = "ogDescription")]
    pub og_description: Option<String>,
    #[serde(rename = "ogUrl")]
    pub og_url: Option<String>,
    #[serde(rename = "ogImage")]
    pub og_image: Option<String>,
    #[serde(rename = "ogAudio")]
    pub og_audio: Option<String>,
    #[serde(rename = "ogDeterminer")]
    pub og_determiner: Option<String>,
    #[serde(rename = "ogLocale")]
    pub og_locale: Option<String>,
    #[serde(rename = "ogLocaleAlternate")]
    pub og_locale_alternate: Option<Vec<String>>,
    #[serde(rename = "ogSiteName")]
    pub og_site_name: Option<String>,
    #[serde(rename = "ogVideo")]
    pub og_video: Option<String>,
    #[serde(rename = "dcTermsCreated")]
    pub dc_terms_created: Option<String>,
    #[serde(rename = "dcDateCreated")]
    pub dc_date_created: Option<String>,
    #[serde(rename = "dcDate")]
    pub dc_date: Option<String>,
    #[serde(rename = "dcTermsType")]
    pub dc_terms_type: Option<String>,
    #[serde(rename = "dcType")]
    pub dc_type: Option<String>,
    #[serde(rename = "dcTermsAudience")]
    pub dc_terms_audience: Option<String>,
    #[serde(rename = "dcTermsSubject")]
    pub dc_terms_subject: Option<String>,
    #[serde(rename = "dcSubject")]
    pub dc_subject: Option<String>,
    #[serde(rename = "dcDescription")]
    pub dc_description: Option<String>,
    #[serde(rename = "dcTermsKeywords")]
    pub dc_terms_keywords: Option<String>,
    #[serde(rename = "modifiedTime")]
    pub modified_time: Option<String>,
    #[serde(rename = "publishedTime")]
    pub published_time: Option<String>,
    #[serde(rename = "articleTag")]
    pub article_tag: Option<String>,
    #[serde(rename = "articleSection")]
    pub article_section: Option<String>,
    #[serde(rename = "sourceURL")]
    pub source_url: Option<String>,
    #[serde(rename = "statusCode")]
    pub status_code: Option<u32>,
    pub error: Option<String>,
    pub site_map: Option<Sitemap>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Sitemap {
    pub changefreq: String,
}

pub async fn crawl(
    crawl_options: CrawlOptions,
    pool: web::Data<Pool>,
    redis_pool: web::Data<RedisPool>,
    dataset_id: uuid::Uuid,
) -> Result<uuid::Uuid, ServiceError> {
    let scrape_id = if let Some(ScrapeOptions::Shopify(_)) = crawl_options.scrape_options {
        uuid::Uuid::nil()
    } else {
        crawl_site(crawl_options.clone())
            .await
            .map_err(|err| ServiceError::BadRequest(format!("Could not crawl site: {}", err)))?
    };

    create_crawl_request(crawl_options, dataset_id, scrape_id, pool, redis_pool).await?;

    Ok(scrape_id)
}

pub async fn get_crawl_request(
    crawl_id: uuid::Uuid,
    pool: web::Data<Pool>,
) -> Result<CrawlRequest, ServiceError> {
    use crate::data::schema::crawl_requests::dsl::*;
    let mut conn = pool
        .get()
        .await
        .map_err(|e| ServiceError::InternalServerError(e.to_string()))?;
    let request = crawl_requests
        .select((
            id,
            url,
            status,
            next_crawl_at,
            interval,
            crawl_options,
            scrape_id,
            dataset_id,
            created_at,
        ))
        .filter(scrape_id.eq(crawl_id))
        .first::<CrawlRequestPG>(&mut conn)
        .await
        .map_err(|e| ServiceError::InternalServerError(e.to_string()))?;

    Ok(request.into())
}

pub async fn get_crawl_request_by_dataset_id_query(
    dataset_id: uuid::Uuid,
    pool: web::Data<Pool>,
) -> Result<Option<CrawlRequest>, ServiceError> {
    use crate::data::schema::crawl_requests::dsl as crawl_requests_table;
    let mut conn = pool
        .get()
        .await
        .map_err(|e| ServiceError::InternalServerError(e.to_string()))?;
    let request: Option<CrawlRequestPG> = crawl_requests_table::crawl_requests
        .filter(crawl_requests_table::dataset_id.eq(dataset_id))
        .select((
            crawl_requests_table::id,
            crawl_requests_table::url,
            crawl_requests_table::status,
            crawl_requests_table::next_crawl_at,
            crawl_requests_table::interval,
            crawl_requests_table::crawl_options,
            crawl_requests_table::scrape_id,
            crawl_requests_table::dataset_id,
            crawl_requests_table::created_at,
        ))
        .first(&mut conn)
        .await
        .optional()
        .map_err(|e| ServiceError::InternalServerError(e.to_string()))?;

    Ok(request.map(|req| req.into()))
}

pub async fn get_crawl_requests_to_rerun(
    pool: web::Data<Pool>,
) -> Result<Vec<CrawlRequest>, ServiceError> {
    use crate::data::schema::crawl_requests::dsl::*;
    let mut conn = pool
        .get()
        .await
        .map_err(|e| ServiceError::InternalServerError(e.to_string()))?;
    let requests = crawl_requests
        .select((
            id,
            url,
            status,
            next_crawl_at,
            interval,
            crawl_options,
            scrape_id,
            dataset_id,
            created_at,
        ))
        .filter(next_crawl_at.le(chrono::Utc::now().naive_utc()))
        .load::<CrawlRequestPG>(&mut conn)
        .await
        .map_err(|e| ServiceError::InternalServerError(e.to_string()))?;
    Ok(requests.into_iter().map(|r| r.into()).collect())
}

pub async fn create_crawl_request(
    crawl_options: CrawlOptions,
    dataset_id: uuid::Uuid,
    scrape_id: uuid::Uuid,
    pool: web::Data<Pool>,
    redis_pool: web::Data<RedisPool>,
) -> Result<uuid::Uuid, ServiceError> {
    use crate::data::schema::crawl_requests::dsl as crawl_requests_table;

    let interval = match crawl_options.interval {
        Some(CrawlInterval::Daily) => std::time::Duration::from_secs(60 * 60 * 24),
        Some(CrawlInterval::Weekly) => std::time::Duration::from_secs(60 * 60 * 24 * 7),
        Some(CrawlInterval::Monthly) => std::time::Duration::from_secs(60 * 60 * 24 * 30),
        None => std::time::Duration::from_secs(60 * 60 * 24),
    };

    let new_crawl_request: CrawlRequestPG = CrawlRequest {
        id: uuid::Uuid::new_v4(),
        url: crawl_options.site_url.clone().unwrap_or_default(),
        status: CrawlStatus::Pending,
        interval,
        next_crawl_at: chrono::Utc::now().naive_utc(),
        crawl_options,
        scrape_id,
        dataset_id,
        created_at: chrono::Utc::now().naive_utc(),
        attempt_number: 0,
    }
    .into();

    let mut conn = pool
        .get()
        .await
        .map_err(|e| ServiceError::InternalServerError(e.to_string()))?;

    diesel::insert_into(crawl_requests_table::crawl_requests)
        .values(&new_crawl_request)
        .execute(&mut conn)
        .await
        .map_err(|e| ServiceError::InternalServerError(e.to_string()))?;

    let serialized_message =
        serde_json::to_string(&CrawlRequest::from(new_crawl_request.clone())).unwrap();
    let mut redis_conn = redis_pool
        .get()
        .await
        .map_err(|e| ServiceError::InternalServerError(e.to_string()))?;

    redis::cmd("lpush")
        .arg("scrape_queue")
        .arg(&serialized_message)
        .query_async::<redis::aio::MultiplexedConnection, usize>(&mut *redis_conn)
        .await
        .map_err(|err| ServiceError::BadRequest(err.to_string()))?;

    Ok(new_crawl_request.scrape_id)
}

pub async fn update_crawl_status(
    crawl_id: uuid::Uuid,
    status: CrawlStatus,
    pool: web::Data<Pool>,
) -> Result<(), ServiceError> {
    use crate::data::schema::crawl_requests::dsl as crawl_requests_table;

    let mut conn = pool
        .get()
        .await
        .map_err(|e| ServiceError::InternalServerError(e.to_string()))?;

    diesel::update(
        crawl_requests_table::crawl_requests.filter(crawl_requests_table::scrape_id.eq(crawl_id)),
    )
    .set(crawl_requests_table::status.eq(status.to_string()))
    .execute(&mut conn)
    .await
    .map_err(|e| ServiceError::InternalServerError(e.to_string()))?;

    Ok(())
}

pub async fn update_next_crawl_at(
    crawl_id: uuid::Uuid,
    next_crawl_at: chrono::NaiveDateTime,
    pool: web::Data<Pool>,
) -> Result<(), ServiceError> {
    use crate::data::schema::crawl_requests::dsl as crawl_requests_table;
    let mut conn = pool
        .get()
        .await
        .map_err(|e| ServiceError::InternalServerError(e.to_string()))?;
    diesel::update(
        crawl_requests_table::crawl_requests.filter(crawl_requests_table::scrape_id.eq(crawl_id)),
    )
    .set(crawl_requests_table::next_crawl_at.eq(next_crawl_at))
    .execute(&mut conn)
    .await
    .map_err(|e| ServiceError::InternalServerError(e.to_string()))?;
    Ok(())
}

pub async fn update_crawl_settings_for_dataset(
    crawl_options: CrawlOptions,
    dataset_id: uuid::Uuid,
    pool: web::Data<Pool>,
    redis_pool: web::Data<RedisPool>,
) -> Result<(), ServiceError> {
    use crate::data::schema::crawl_requests::dsl as crawl_requests_table;
    let mut conn = pool
        .get()
        .await
        .map_err(|e| ServiceError::InternalServerError(e.to_string()))?;

    let prev_crawl_req = crawl_requests_table::crawl_requests
        .select((
            crawl_requests_table::id,
            crawl_requests_table::url,
            crawl_requests_table::status,
            crawl_requests_table::next_crawl_at,
            crawl_requests_table::interval,
            crawl_requests_table::crawl_options,
            crawl_requests_table::scrape_id,
            crawl_requests_table::dataset_id,
            crawl_requests_table::created_at,
        ))
        .filter(crawl_requests_table::dataset_id.eq(dataset_id))
        .first::<CrawlRequestPG>(&mut conn)
        .await
        .optional()?;

    if let Some(ref url) = crawl_options.site_url {
        diesel::update(
            crawl_requests_table::crawl_requests
                .filter(crawl_requests_table::dataset_id.eq(dataset_id)),
        )
        .set(crawl_requests_table::url.eq(url))
        .execute(&mut conn)
        .await
        .map_err(|e| {
            log::error!("Error updating url on crawl_requests: {:?}", e);
            ServiceError::InternalServerError("Error updating url on crawl_requests".to_string())
        })?;
    }

    if let Some(interval) = crawl_options.interval.clone() {
        let interval = match interval {
            CrawlInterval::Daily => std::time::Duration::from_secs(60 * 60 * 24),
            CrawlInterval::Weekly => std::time::Duration::from_secs(60 * 60 * 24 * 7),
            CrawlInterval::Monthly => std::time::Duration::from_secs(60 * 60 * 24 * 30),
        };
        diesel::update(
            crawl_requests_table::crawl_requests
                .filter(crawl_requests_table::dataset_id.eq(dataset_id)),
        )
        .set(crawl_requests_table::interval.eq(interval.as_secs() as i32))
        .execute(&mut conn)
        .await
        .map_err(|e| {
            log::error!("Error updating interval on crawl_requests: {:?}", e);
            ServiceError::InternalServerError(
                "Error updating interval on crawl_requests".to_string(),
            )
        })?;
    }

    let merged_options = if let Some(prev_crawl_req) = prev_crawl_req {
        let previous_crawl_options: CrawlOptions =
            serde_json::from_value(prev_crawl_req.crawl_options)
                .map_err(|e| ServiceError::InternalServerError(e.to_string()))?;
        crawl_options.merge(previous_crawl_options)
    } else {
        crawl_options
    };

    diesel::update(
        crawl_requests_table::crawl_requests
            .filter(crawl_requests_table::dataset_id.eq(dataset_id)),
    )
    .set(crawl_requests_table::crawl_options.eq(
        serde_json::to_value(merged_options.clone()).map_err(|e| {
            log::error!("Failed to serialize crawl options: {:?}", e);
            ServiceError::BadRequest("Failed to serialize crawl options".to_string())
        })?,
    ))
    .execute(&mut conn)
    .await
    .map_err(|e| {
        log::error!("Error updating crawl options on crawl_requests: {:?}", e);
        ServiceError::InternalServerError(
            "Error updating crawl options on crawl_requests".to_string(),
        )
    })?;

    crawl(
        merged_options.clone(),
        pool.clone(),
        redis_pool.clone(),
        dataset_id,
    )
    .await?;

    Ok(())
}

pub async fn update_scrape_id(
    scrape_id: uuid::Uuid,
    new_scrape_id: uuid::Uuid,
    pool: web::Data<Pool>,
) -> Result<CrawlRequest, ServiceError> {
    use crate::data::schema::crawl_requests::dsl as crawl_requests_table;
    let mut conn = pool
        .get()
        .await
        .map_err(|e| ServiceError::InternalServerError(e.to_string()))?;
    let updated_request = diesel::update(
        crawl_requests_table::crawl_requests.filter(crawl_requests_table::scrape_id.eq(scrape_id)),
    )
    .set(crawl_requests_table::scrape_id.eq(new_scrape_id))
    .returning(CrawlRequestPG::as_returning())
    .get_result(&mut conn)
    .await
    .map_err(|e| ServiceError::InternalServerError(e.to_string()))?;

    Ok(updated_request.into())
}

pub async fn get_crawl_from_firecrawl(scrape_id: uuid::Uuid) -> Result<IngestResult, ServiceError> {
    log::info!("Getting crawl from firecrawl");

    let firecrawl_url =
        std::env::var("FIRECRAWL_URL").unwrap_or_else(|_| "https://api.firecrawl.dev".to_string());
    let firecrawl_api_key = std::env::var("FIRECRAWL_API_KEY").unwrap_or_else(|_| "".to_string());
    let mut firecrawl_url = format!("{}/v1/crawl/{}", firecrawl_url, scrape_id);

    let mut collected_docs: Vec<Option<Document>> = vec![];
    let mut resp = None;

    let client = reqwest::Client::new();

    while resp.is_none() {
        let response = client
            .get(&firecrawl_url)
            .header("Authorization", format!("Bearer {}", firecrawl_api_key))
            .send()
            .await
            .map_err(|e| {
                log::error!("Error sending request to firecrawl: {:?}", e);
                ServiceError::InternalServerError("Error sending request to firecrawl".to_string())
            })?;

        if !response.status().is_success() {
            log::error!(
                "Error getting response from firecrawl: {:?}",
                response.text().await
            );
            return Err(ServiceError::InternalServerError(
                "Error getting response from firecrawl".to_string(),
            ));
        };

        let ingest_result = response.json::<IngestResult>().await.map_err(|e| {
            log::error!("Error parsing response from firecrawl: {:?}", e);
            ServiceError::InternalServerError("Error parsing response from firecrawl".to_string())
        })?;

        if ingest_result.status != Status::Completed {
            log::info!("Crawl status: {:?}", ingest_result.status);
            return Ok(ingest_result);
        }

        let cur_docs = ingest_result.clone().data.unwrap_or_default();
        collected_docs.extend(cur_docs);

        if let Some(ref next_ingest_result) = ingest_result.next {
            let next_ingest_result = next_ingest_result.replace("https://", "http://");

            log::info!(
                "Next ingest url: {} | prev {}",
                next_ingest_result,
                firecrawl_url
            );
            if next_ingest_result == firecrawl_url {
                log::info!("Breaking loop");
                resp = Some(ingest_result.clone());
                break;
            }

            firecrawl_url = next_ingest_result;
        } else {
            resp = Some(ingest_result.clone());
        }
    }

    match resp {
        Some(resp) => Ok(IngestResult {
            status: resp.status,
            completed: resp.completed,
            total: resp.total,
            credits_used: resp.credits_used,
            expires_at: resp.expires_at,
            next: None,
            data: Some(collected_docs),
        }),
        None => Err(ServiceError::InternalServerError(
            "Error getting response from firecrawl".to_string(),
        )),
    }
}

pub async fn crawl_site(crawl_options: CrawlOptions) -> Result<uuid::Uuid, ServiceError> {
    let firecrawl_url =
        std::env::var("FIRECRAWL_URL").unwrap_or_else(|_| "https://api.firecrawl.dev".to_string());
    let firecrawl_api_key = std::env::var("FIRECRAWL_API_KEY").unwrap_or_else(|_| "".to_string());
    let firecrawl_url = format!("{}/v1/crawl", firecrawl_url);
    let client = reqwest::Client::new();
    let response = client
        .post(&firecrawl_url)
        .json(&FirecrawlCrawlRequest::from(crawl_options))
        .header("Authorization", format!("Bearer {}", firecrawl_api_key))
        .send()
        .await
        .map_err(|e| {
            log::error!("Error sending request to firecrawl: {:?}", e);
            ServiceError::InternalServerError("Error sending request to firecrawl".to_string())
        })?;

    if response.status().is_success() {
        let json = response.json::<serde_json::Value>().await.map_err(|e| {
            log::error!("Error parsing response from firecrawl: {:?}", e);
            ServiceError::InternalServerError("Error parsing response from firecrawl".to_string())
        })?;

        Ok(json.get("id").unwrap().as_str().unwrap().parse().unwrap())
    } else {
        log::error!(
            "Error getting response from firecrawl: {:?}",
            response.text().await
        );
        Err(ServiceError::InternalServerError(
            "Error getting response from firecrawl".to_string(),
        ))
    }
}

pub fn get_tags(url: String) -> Vec<String> {
    if let Ok(parsed_url) = Url::parse(&url) {
        let path_parts: Vec<&str> = parsed_url.path().split('/').collect();
        return path_parts
            .iter()
            .filter_map(|part| {
                if !part.is_empty() {
                    Some(part.to_string())
                } else {
                    None
                }
            })
            .collect();
    }
    Vec::new()
}

pub fn chunk_html(html: &str) -> Vec<(String, String)> {
    let re = Regex::new(r"(?i)<h[1-6].*?>").unwrap();
    let mut chunks = Vec::new();
    let mut current_chunk = String::new();
    let mut last_end = 0;
    let mut short_chunk: Option<String> = None;

    for cap in re.find_iter(html) {
        if last_end != cap.start() {
            current_chunk.push_str(&html[last_end..cap.start()]);
        }

        if !current_chunk.is_empty() {
            let trimmed_chunk = current_chunk.trim().to_string();

            if let Some(prev_short_chunk) = short_chunk.take() {
                current_chunk = format!("{} {}", prev_short_chunk, trimmed_chunk);
            } else {
                current_chunk = trimmed_chunk;
            }

            if convert_html_to_text(&current_chunk)
                .split_whitespace()
                .count()
                > 5
            {
                let heading = extract_first_heading(&current_chunk);
                chunks.push((heading, current_chunk));
            } else {
                short_chunk = Some(current_chunk);
            }
        }

        current_chunk = cap.as_str().to_string();
        last_end = cap.end();
    }

    if last_end < html.len() {
        current_chunk.push_str(&html[last_end..]);
    }

    if !current_chunk.is_empty() {
        let trimmed_chunk = current_chunk.trim().to_string();

        if let Some(prev_short_chunk) = short_chunk.take() {
            current_chunk = format!("{} {}", prev_short_chunk, trimmed_chunk);
        } else {
            current_chunk = trimmed_chunk;
        }

        let heading = extract_first_heading(&current_chunk);
        chunks.push((heading, current_chunk));
    } else if let Some(last_short_chunk) = short_chunk {
        let heading = extract_first_heading(&last_short_chunk);
        chunks.push((heading, last_short_chunk));
    }

    chunks
}

fn extract_first_heading(html: &str) -> String {
    let fragment = Html::parse_fragment(html);
    let heading_selector = Selector::parse("h1, h2, h3, h4, h5, h6").unwrap();

    fragment
        .select(&heading_selector)
        .next()
        .map(|element| element.text().collect::<String>())
        .unwrap_or_default()
}
