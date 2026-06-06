import { useEffect, useState } from 'react';
import { getRuntimeConfig, apiUrl, API_BASE_URL } from './config';
import Header from './components/Header';

function Row({ label, value, mono = true }) {
  return (
    <tr>
      <td
        style={{
          padding: '10px 16px',
          color: 'var(--color-text-secondary, #94a3b8)',
          whiteSpace: 'nowrap',
          fontWeight: 500,
          width: '220px',
          verticalAlign: 'top',
        }}
      >
        {label}
      </td>
      <td
        style={{
          padding: '10px 16px',
          fontFamily: mono ? 'monospace' : 'inherit',
          fontSize: mono ? '0.85rem' : 'inherit',
          wordBreak: 'break-all',
          color: 'var(--color-text, #e2e8f0)',
        }}
      >
        {value || <span style={{ color: '#64748b', fontStyle: 'italic' }}>not configured</span>}
      </td>
    </tr>
  );
}

function Section({ title, children }) {
  return (
    <section style={{ marginBottom: '32px' }}>
      <h2
        style={{
          fontSize: '0.75rem',
          fontWeight: 700,
          letterSpacing: '0.1em',
          textTransform: 'uppercase',
          color: 'var(--color-text-secondary, #64748b)',
          marginBottom: '8px',
          paddingLeft: '16px',
        }}
      >
        {title}
      </h2>
      <table
        style={{
          width: '100%',
          borderCollapse: 'collapse',
          background: 'var(--color-surface, #1e293b)',
          borderRadius: '12px',
          overflow: 'hidden',
          border: '1px solid var(--color-border, rgba(255,255,255,0.08))',
        }}
      >
        <tbody>{children}</tbody>
      </table>
    </section>
  );
}

export default function About({
  theme,
  onToggleTheme,
  stellarNetwork,
  onChangeStellarNetwork,
  walletAddress,
  walletBalance,
  isWalletLoading,
  isWalletBalanceLoading,
  onConnectWallet,
  onDisconnectWallet,
}) {
  const [config, setConfig] = useState(() => getRuntimeConfig());

  useEffect(() => {
    setConfig(getRuntimeConfig());
  }, [stellarNetwork]);

  const { stellar, contracts, sources } = config;

  return (
    <div
      style={{
        minHeight: '100vh',
        background: 'var(--color-bg, #0f172a)',
        color: 'var(--color-text, #e2e8f0)',
      }}
    >
      <Header
        theme={theme}
        onToggleTheme={onToggleTheme}
        stellarNetwork={stellarNetwork}
        onChangeStellarNetwork={onChangeStellarNetwork}
        walletAddress={walletAddress}
        walletBalance={walletBalance}
        isWalletLoading={isWalletLoading}
        isWalletBalanceLoading={isWalletBalanceLoading}
        onConnectWallet={onConnectWallet}
        onDisconnectWallet={onDisconnectWallet}
      />

      <main
        style={{
          maxWidth: '760px',
          margin: '0 auto',
          padding: '80px 24px 60px',
        }}
      >
        <h1
          style={{
            fontSize: '1.75rem',
            fontWeight: 700,
            marginBottom: '8px',
            color: 'var(--color-text, #e2e8f0)',
          }}
        >
          Runtime Configuration
        </h1>
        <p
          style={{
            color: 'var(--color-text-secondary, #94a3b8)',
            marginBottom: '40px',
            fontSize: '0.9rem',
          }}
        >
          Active environment and contract settings. Sources show where each value originated (env,
          backend, or dev-switcher).
        </p>

        <Section title="API">
          <Row label="API Base URL" value={API_BASE_URL || window.location.origin} />
          <Row label="Campaigns endpoint" value={apiUrl('/api/v1/campaigns')} />
          <Row label="Config endpoint" value={apiUrl('/api/v1/config')} />
        </Section>

        <Section title="Stellar Network">
          <Row label="Network" value={stellar.network} mono={false} />
          <Row label="Network passphrase" value={stellar.networkPassphrase} />
          <Row label="Soroban RPC URL" value={stellar.sorobanRpcUrl} />
          <Row label="Horizon URL" value={stellar.horizonUrl} />
          <Row label="Source" value={sources.stellar} mono={false} />
        </Section>

        <Section title="Contract IDs">
          <Row label="Rewards contract" value={contracts.rewards} />
          <Row label="Campaign contract" value={contracts.campaign} />
          <Row label="Source" value={sources.contracts} mono={false} />
        </Section>
      </main>
    </div>
  );
}
