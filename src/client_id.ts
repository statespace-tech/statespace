import { randomUUID } from 'crypto';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'fs';
import { homedir } from 'os';
import { join } from 'path';

export function getClientId(): string {
  const dir = join(homedir(), '.statespace');
  const file = join(dir, 'client_id');
  if (existsSync(file)) {
    return readFileSync(file, 'utf8').trim();
  }
  const id = randomUUID();
  mkdirSync(dir, { recursive: true });
  writeFileSync(file, id, 'utf8');
  return id;
}
