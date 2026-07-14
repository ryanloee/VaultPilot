/**
 * Regression test for issue #2839
 *
 * Bug: expo core was pinned to SDK 57 (~57.0.4) but several expo-* native
 * modules were still locked to SDK 56 in mobile/package.json
 * (expo-file-system ~56.0.8, expo-intent-launcher ~56.0.4, expo-sqlite ^56.0.5).
 * EAS Build / expo prebuild would then resolve native modules whose version
 * mismatches the installed SDK, causing pod install / gradle conflicts and
 * native bridge runtime errors.
 *
 * Fix: align every officially-published expo-* module to the same major
 * version as `expo` and keep package-lock.json in sync.
 *
 * This guard reads package.json / package-lock.json directly (no RN runtime),
 * so it runs under the plain `node` jest environment.
 */
import fs from 'fs';
import path from 'path';

const MOBILE_ROOT = path.resolve(__dirname, '../../../');

function readJson(rel: string): any {
  return JSON.parse(fs.readFileSync(path.join(MOBILE_ROOT, rel), 'utf8'));
}

/** Extract the leading major version from a semver range like "~57.0.4". */
function major(range: string): number {
  const m = range.match(/(\d+)\./);
  if (!m) throw new Error(`cannot parse major from range: ${range}`);
  return parseInt(m[1], 10);
}

/**
 * Third-party expo-* packages that intentionally lag the Expo SDK cadence
 * because they publish on their own schedule. They declare `expo: "*"` as a
 * peer dependency, so they remain compatible across SDK majors.
 *
 * expo-speech-recognition has no 57.x release published (latest is 56.0.1),
 * so it is explicitly allowed to differ from the expo major.
 */
const ALLOWED_LAGGING = new Set<string>(['expo-speech-recognition']);

describe('issue #2839 — expo SDK module alignment', () => {
  const pkg = readJson('package.json');
  const deps: Record<string, string> = pkg.dependencies ?? {};
  const expoMajor = major(deps['expo']);

  it('expo core is pinned to a known SDK major', () => {
    expect(expoMajor).toBeGreaterThanOrEqual(57);
  });

  it('every officially-published expo-* module shares the expo SDK major', () => {
    const offenders: string[] = [];
    for (const [name, range] of Object.entries(deps)) {
      if (name === 'expo') continue;
      if (!name.startsWith('expo-')) continue;
      if (ALLOWED_LAGGING.has(name)) continue;
      if (major(range) !== expoMajor) {
        offenders.push(`${name}@${range} (expected major ${expoMajor})`);
      }
    }
    expect(offenders).toEqual([]);
  });

  it('the previously-mismatched modules are pinned to the expo major', () => {
    for (const name of ['expo-file-system', 'expo-intent-launcher', 'expo-sqlite']) {
      expect(deps[name]).toBeDefined();
      expect(major(deps[name])).toBe(expoMajor);
    }
  });

  it('package-lock.json resolves the fixed modules to the expo major', () => {
    const lock = readJson('package-lock.json');
    const pkgs = lock.packages ?? {};
    for (const name of ['expo-file-system', 'expo-intent-launcher', 'expo-sqlite']) {
      const entry = pkgs[`node_modules/${name}`];
      expect(entry).toBeDefined();
      expect(major(entry.version)).toBe(expoMajor);
    }
  });
});
