// JNI shim untuk menjalankan Node.js (nodejs-mobile) dari NodeService.
//
// node::Start hanya aman dipanggil sekali per proses. NodeService berjalan
// di proses :node dan membunuh prosesnya saat stop, sehingga start
// berikutnya selalu memakai proses baru.

#include <jni.h>
#include <node.h>

#include <android/log.h>
#include <fcntl.h>
#include <unistd.h>

#include <string>
#include <vector>

namespace {

constexpr const char* kLogTag = "WebForgeNode";

bool JStringToStdString(JNIEnv* env, jstring value, std::string* out) {
  if (value == nullptr) {
    out->clear();
    return true;
  }
  const char* chars = env->GetStringUTFChars(value, nullptr);
  if (chars == nullptr) {
    return false;
  }
  *out = chars;
  env->ReleaseStringUTFChars(value, chars);
  return true;
}

bool RedirectStdoutStderrToLog(const std::string& log_path) {
  if (log_path.empty()) {
    return true;
  }
  int fd = open(log_path.c_str(), O_WRONLY | O_CREAT | O_APPEND, 0644);
  if (fd < 0) {
    __android_log_print(ANDROID_LOG_ERROR, kLogTag,
                        "Gagal membuka log Node: %s", log_path.c_str());
    return false;
  }
  if (dup2(fd, STDOUT_FILENO) < 0 || dup2(fd, STDERR_FILENO) < 0) {
    __android_log_print(ANDROID_LOG_ERROR, kLogTag,
                        "Gagal mengalihkan stdout/stderr ke log Node");
    close(fd);
    return false;
  }
  if (fd > STDERR_FILENO) {
    close(fd);
  }
  return true;
}

}  // namespace

extern "C" JNIEXPORT jint JNICALL
Java_com_webforge_mobile_NodeService_nativeStartNode(
    JNIEnv* env, jobject /* thiz */, jobjectArray args, jstring cwd,
    jstring log_path) {
  std::string cwd_str;
  std::string log_path_str;
  if (!JStringToStdString(env, cwd, &cwd_str) ||
      !JStringToStdString(env, log_path, &log_path_str)) {
    __android_log_print(ANDROID_LOG_ERROR, kLogTag,
                        "Gagal membaca cwd/logPath dari JNI");
    return 127;
  }

  if (!cwd_str.empty() && chdir(cwd_str.c_str()) != 0) {
    __android_log_print(ANDROID_LOG_ERROR, kLogTag,
                        "Gagal chdir ke folder proyek Node: %s",
                        cwd_str.c_str());
    RedirectStdoutStderrToLog(log_path_str);
    return 126;
  }
  if (!RedirectStdoutStderrToLog(log_path_str)) {
    return 126;
  }

  std::vector<std::string> storage;
  storage.emplace_back("node");
  if (args != nullptr) {
    const jsize count = env->GetArrayLength(args);
    for (jsize i = 0; i < count; ++i) {
      auto item = static_cast<jstring>(env->GetObjectArrayElement(args, i));
      std::string arg;
      if (!JStringToStdString(env, item, &arg)) {
        if (item != nullptr) {
          env->DeleteLocalRef(item);
        }
        __android_log_print(ANDROID_LOG_ERROR, kLogTag,
                            "Gagal membaca argumen Node ke-%d", static_cast<int>(i));
        return 127;
      }
      if (item != nullptr) {
        env->DeleteLocalRef(item);
      }
      storage.push_back(arg);
    }
  }

  std::vector<char*> argv;
  argv.reserve(storage.size() + 1);
  for (std::string& value : storage) {
    argv.push_back(const_cast<char*>(value.c_str()));
  }
  argv.push_back(nullptr);

  const int exit_code = node::Start(static_cast<int>(storage.size()), argv.data());
  return static_cast<jint>(exit_code);
}
