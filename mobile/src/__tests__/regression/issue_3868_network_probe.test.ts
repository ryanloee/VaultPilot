/**
 * Regression tests for #3868: 连通性探测用 gstatic.com，国内网络不可达 →
 * 误判离线（离线横幅常驻、待同步自动 flush 失效）。
 *
 * Bug: checkConnection probed https://www.gstatic.com/generate_204 (Google),
 * unreachable in mainland China → isOnline always false despite working
 * network, so OfflineBanner stayed visible and offline→online flush never fired.
 *
 * Fix: probeConnectivity prefers the configured backend /health (pingBackend —
 * same network path as sync traffic, reachable in China); gstatic is only used
 * as a fallback when no backend is configured.
 */

import { probeConnectivity } from '../../utils/networkState';
import { pingBackend } from '../../services/sync';

jest.mock('../../services/sync', () => ({
  pingBackend: jest.fn(),
}));

const mockPingBackend = pingBackend as jest.MockedFunction<typeof pingBackend>;

const mockFetch = jest.fn();
(globalThis as any).fetch = mockFetch;

beforeEach(() => {
  jest.clearAllMocks();
  mockFetch.mockReset();
  mockPingBackend.mockReset();
});

describe('probeConnectivity prefers backend /health (#3868)', () => {
  it('returns true via backend /health without touching gstatic', async () => {
    mockPingBackend.mockResolvedValue(true);

    const online = await probeConnectivity();
    expect(online).toBe(true);
    // Backend probe succeeded → no public-endpoint fallback needed.
    expect(mockFetch).not.toHaveBeenCalled();
  });

  it('falls back to gstatic only when no backend is configured / unreachable', async () => {
    mockPingBackend.mockResolvedValue(false);
    mockFetch.mockResolvedValueOnce({ ok: true });

    const online = await probeConnectivity();
    expect(online).toBe(true);
    expect(mockFetch).toHaveBeenCalledWith(
      'https://www.gstatic.com/generate_204',
      expect.objectContaining({ method: 'HEAD' }),
    );
  });

  it('returns false when both backend and public endpoint fail', async () => {
    mockPingBackend.mockResolvedValue(false);
    mockFetch.mockRejectedValueOnce(new Error('Network error'));

    const online = await probeConnectivity();
    expect(online).toBe(false);
  });

  it('returns false when public endpoint returns non-ok', async () => {
    mockPingBackend.mockResolvedValue(false);
    mockFetch.mockResolvedValueOnce({ ok: false, status: 503 });

    const online = await probeConnectivity();
    expect(online).toBe(false);
  });
});
