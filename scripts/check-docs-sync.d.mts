export interface DocumentationCheckOptions {
  root?: string;
  checkExternal?: boolean;
  fetchImpl?: (url: string, init: RequestInit) => Promise<Response>;
  externalTimeoutMs?: number;
}

export interface DocumentationCheckResult {
  errors: string[];
  documents: number;
  notes: number;
  links: number;
  externalLinks: number;
}

export function checkDocuments(options?: DocumentationCheckOptions): Promise<DocumentationCheckResult>;
