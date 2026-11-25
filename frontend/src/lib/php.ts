import type { Project } from './types';

/** Framework PHP yang dikenali deteksi Core (core/src/framework.rs). */
const PHP_FRAMEWORKS = new Set([
  'Laravel', 'CodeIgniter 4', 'CodeIgniter 3', 'WordPress', 'Symfony', 'Slim', 'Native PHP',
]);

/** True bila proyek terdeteksi PHP dari data deteksi Core. */
export function isPhpProject(project: Project | null | undefined): boolean {
  const fw = project?.framework;
  if (!fw) return false;
  return fw.language === 'PHP' || fw.recommended_runtime === 'php' || PHP_FRAMEWORKS.has(fw.framework);
}

export interface PhpRequirement {
  /** Label yang ditampilkan, mis. "mysqli atau pdo_mysql". */
  name: string;
  /** Ekstensi yang memenuhi kebutuhan ini (salah satu cukup). */
  anyOf: string[];
}

function req(name: string, anyOf?: string[]): PhpRequirement {
  return { name, anyOf: anyOf ?? [name] };
}

/**
 * Kebutuhan ekstensi umum per framework PHP. Daftar ini adalah kebutuhan
 * umum yang dikenal publik untuk tiap framework; status terpenuhi/kurang
 * selalu dibandingkan dengan daftar nyata dari /api/runtimes/php/info.
 */
export function requiredPhpExtensions(framework: string): PhpRequirement[] {
  switch (framework) {
    case 'Laravel':
      return ['mbstring', 'openssl', 'pdo', 'tokenizer', 'xml', 'ctype', 'json', 'bcmath', 'fileinfo'].map((n) => req(n));
    case 'WordPress':
      return [req('mysqli atau pdo_mysql', ['mysqli', 'pdo_mysql']), req('gd'), req('curl'), req('mbstring'), req('xml'), req('zip')];
    case 'CodeIgniter 4':
      return ['intl', 'mbstring', 'json', 'curl', 'xml'].map((n) => req(n));
    case 'CodeIgniter 3':
      return ['mbstring', 'json', 'curl'].map((n) => req(n));
    case 'Symfony':
      return ['ctype', 'iconv', 'json', 'xml', 'mbstring', 'tokenizer'].map((n) => req(n));
    case 'Slim':
      return ['json', 'mbstring', 'xml'].map((n) => req(n));
    default:
      return [];
  }
}

export interface PhpRequirementStatus extends PhpRequirement {
  satisfied: boolean;
}

/** Bandingkan kebutuhan framework dengan ekstensi nyata (tidak peka huruf besar/kecil). */
export function evaluatePhpRequirements(framework: string, extensions: string[]): PhpRequirementStatus[] {
  const have = new Set(extensions.map((e) => e.trim().toLowerCase()));
  return requiredPhpExtensions(framework).map((r) => ({
    ...r,
    satisfied: r.anyOf.some((e) => have.has(e.toLowerCase())),
  }));
}
