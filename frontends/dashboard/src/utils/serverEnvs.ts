import { DatasetConfigurationDTO } from "trieve-ts-sdk";

const bm25Active = import.meta.env.VITE_BM25_ACTIVE as unknown as string;

export const defaultServerEnvsConfiguration: DatasetConfigurationDTO = {
  LLM_BASE_URL: "",
  LLM_DEFAULT_MODEL: "",
  EMBEDDING_BASE_URL: "https://embedding.trieve.ai",
  EMBEDDING_MODEL_NAME: "jina-base-en",
  MESSAGE_TO_QUERY_PROMPT: "",
  RAG_PROMPT: "",
  EMBEDDING_SIZE: 768,
  N_RETRIEVALS_TO_INCLUDE: 8,
  FULLTEXT_ENABLED: true,
  SEMANTIC_ENABLED: true,
  EMBEDDING_QUERY_PREFIX: "Search for: ",
  USE_MESSAGE_TO_QUERY_PROMPT: false,
  FREQUENCY_PENALTY: null,
  TEMPERATURE: null,
  PRESENCE_PENALTY: null,
  STOP_TOKENS: null,
  MAX_TOKENS: null,
  INDEXED_ONLY: false,
  LOCKED: false,
  SYSTEM_PROMPT: null,
  MAX_LIMIT: 10000,
  BM25_ENABLED: bm25Active == "true",
  BM25_B: 0.75,
  BM25_K: 1.2,
  BM25_AVG_LEN: 256,
};
