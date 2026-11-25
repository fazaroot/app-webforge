/** @type {import('tailwindcss').Config} */
export default {
  darkMode: 'class',
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  theme: {
    extend: {
      colors: {
        primary: '#5576E8',
        appbg: '#F3F5FA',
        surface: '#FFFFFF',
        ink: '#20283A',
        success: '#2EAD79',
        warning: '#E9A23B',
        danger: '#DC5866',
        darkbg: '#101522',
        darksurface: '#1A2233',
        darkink: '#E9EDF6',
      },
    },
  },
  plugins: [],
};
