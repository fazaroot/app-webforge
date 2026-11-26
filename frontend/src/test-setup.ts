import '@testing-library/jest-dom/vitest';
import { configure } from '@testing-library/react';

// Mesin CI/VM bersama bisa lambat; timeout async default 1 detik membuat
// findBy/waitFor flaky. 10 detik tetap gagal cepat untuk regresi nyata.
configure({ asyncUtilTimeout: 10000 });
