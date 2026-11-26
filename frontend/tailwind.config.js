/** @type {import('tailwindcss').Config} */
export default {
  darkMode: 'class',
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  theme: {
    extend: {
      colors: {
        primary: '#0284c7',
        appbg: '#e8f6fd',
        surface: '#ffffff',
        ink: '#0a2438',
        success: '#2EAD79',
        warning: '#E9A23B',
        danger: '#DC5866',
        darkbg: '#04121f',
        darksurface: '#0a2a44',
        darkink: '#e6f4ff',
      },
    },
  },
  plugins: [],
};
