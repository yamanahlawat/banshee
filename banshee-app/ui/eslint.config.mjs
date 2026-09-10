import js from '@eslint/js';
import prettier from 'eslint-config-prettier';
import svelte from 'eslint-plugin-svelte';
import unusedImports from 'eslint-plugin-unused-imports';
import globals from 'globals';
import ts from 'typescript-eslint';

const REACHES_THE_MOCKS =
  'Only src/lib/tauri.ts may reach the mocks, with an `import()` behind import.meta.env.DEV. Any other import ships them to users.';

export default ts.config(
  js.configs.recommended,
  ...ts.configs.recommended,
  ...svelte.configs['flat/recommended'],

  // Last, so it turns off every rule Prettier already decides.
  prettier,
  ...svelte.configs['flat/prettier'],

  {
    languageOptions: {
      globals: { ...globals.browser, ...globals.es2021 },
    },
  },

  {
    plugins: { 'unused-imports': unusedImports },
    rules: {
      'unused-imports/no-unused-imports': 'error',
      // Off in favour of the plugin's own rule below: both report an unused
      // import, and together they report every one of them twice.
      '@typescript-eslint/no-unused-vars': 'off',
      'unused-imports/no-unused-vars': [
        'error',
        {
          argsIgnorePattern: '^_',
          varsIgnorePattern: '^_',
          caughtErrorsIgnorePattern: '^_',
        },
      ],
      '@typescript-eslint/consistent-type-imports': [
        'error',
        { prefer: 'type-imports', fixStyle: 'inline-type-imports' },
      ],
      // The window speaks through the daemon, not through the console.
      'no-console': ['warn', { allow: ['warn', 'error'] }],
      'prefer-const': 'error',
    },
  },

  // TypeScript inside the script blocks. The two disabled rules are Svelte
  // idioms a general JS linter reads as dead code: `$: (a, (b = c))` exists to
  // depend on `a`, and a memo assigned in a reactive block is read on its next
  // run, not below the assignment.
  {
    files: ['**/*.svelte'],
    languageOptions: { parserOptions: { parser: ts.parser } },
    rules: {
      '@typescript-eslint/no-unused-expressions': 'off',
      'no-useless-assignment': 'off',
    },
  },

  {
    files: ['**/*.test.ts'],
    languageOptions: { globals: globals.node },
  },

  {
    files: ['scripts/**/*.mjs'],
    languageOptions: { globals: globals.node },
    rules: { 'no-console': 'off' },
  },

  // The tests read the same captured replies, and vitest bundles nothing. Two
  // rules cover one boundary, because `no-restricted-imports` reads a static
  // specifier only.
  {
    files: ['src/**/*.ts', 'src/**/*.svelte'],
    ignores: ['src/**/*.test.ts'],
    rules: {
      'no-restricted-imports': [
        'error',
        { patterns: [{ group: ['**/mocks', '**/mocks/*'], message: REACHES_THE_MOCKS }] },
      ],
      'no-restricted-syntax': [
        'error',
        {
          selector: 'ImportExpression > Literal[value=/(^|\\/)mocks(\\/|$)/]',
          message: REACHES_THE_MOCKS,
        },
      ],
    },
  },

  // The bridge's own way in is an `import()`, and only that form is exempt. A
  // static import here would ship the mocks like one anywhere else.
  {
    files: ['src/lib/tauri.ts'],
    rules: { 'no-restricted-syntax': 'off' },
  },

  { ignores: ['dist/**', 'node_modules/**'] },
);
