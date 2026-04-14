import { TelescopeIcon } from '@primer/octicons-react';
import { Button } from '@primer/react';
import RepoHeader, { type Tab } from '../components/RepoHeader';
import { useRepoParam } from '../hooks/useRepoParam';

interface PlaceholderProps {
  tabName: Tab;
  title: string;
  description: string;
  actionText?: string;
}

export default function PlaceholderPage({ tabName, title, description, actionText }: PlaceholderProps) {
  const repo = useRepoParam();

  return (
    <div>
      <RepoHeader repo={repo} currentTab={tabName} />
      <div style={{ padding: '48px', textAlign: 'center', maxWidth: '800px', margin: '40px auto 0 auto', border: '1px solid var(--border-default)', borderRadius: '6px', backgroundColor: 'var(--bg-default)' }}>
        <div style={{ color: 'var(--fg-muted)', marginBottom: '16px', display: 'flex', justifyContent: 'center' }}>
          <TelescopeIcon size={40} />
        </div>
        <h2 style={{ fontSize: '24px', fontWeight: 600, color: 'var(--fg-default)', margin: '0 0 8px 0' }}>
          {title}
        </h2>
        <p style={{ color: 'var(--fg-muted)', fontSize: '16px', margin: '0 0 24px 0', lineHeight: 1.5 }}>
          {description}
        </p>
        <div style={{ display: 'flex', justifyContent: 'center', gap: '8px' }}>
          <Button variant="primary">{actionText || 'Get started'}</Button>
          <Button variant="invisible">Learn more</Button>
        </div>
      </div>
    </div>
  );
}
