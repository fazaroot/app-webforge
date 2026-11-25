import type { CapacitorConfig } from '@capacitor/cli';

const config: CapacitorConfig = {
  appId: 'com.webforge.mobile',
  appName: 'WebForge Mobile',
  webDir: 'frontend/dist',
  plugins: {
    StatusBar: {
      overlaysWebView: false,
      backgroundColor: '#5576E8',
    },
    SplashScreen: {
      backgroundColor: '#5576E8',
    },
  },
};

export default config;
