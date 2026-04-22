# Contributing to Statespace

Thanks for your interest in contributing. This guide covers how to build, test, and submit changes.

## Requirements

- Node.js **18+**
- npm or another package manager

## Getting started

1. Fork and clone:
   ```bash
   git clone https://github.com/<your-username>/statespace.git
   cd statespace
   ```
2. Install dependencies:
   ```bash
   npm install
   ```
3. Build:
   ```bash
   npm run build
   ```
   The compiled CLI lands in `dist/cli.js`.

## Development workflow

Edit TypeScript source in `src/`, then rebuild:

```bash
npm run build
node dist/cli.js search "your query"
```

To test against a local backend, pass `--url`:

```bash
node dist/cli.js search "redis" --url http://localhost:3000
```

## Submitting changes

1. Create a branch from `main`.
2. Keep commits focused and scoped.
3. Open a PR with a clear description and motivation.

## License

Contributions are accepted under the [MIT License](LICENSE).
