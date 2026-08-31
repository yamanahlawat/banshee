import js from '@eslint/js';
import prettier from 'eslint-config-prettier';
import svelte from 'eslint-plugin-svelte';
import unusedImports from 'eslint-plugin-unused-imports';
import globals from 'globals';
import ts from 'typescript-eslint';

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

  { ignores: ['dist/**', 'node_modules/**', 'src/fixtures/**'] },
);
