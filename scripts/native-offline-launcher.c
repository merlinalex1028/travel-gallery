// QA launcher: applies a per-process network denial without changing OS settings.
#include <limits.h>
#include <mach-o/dyld.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

int main(void) {
    char executable[PATH_MAX], resolved[PATH_MAX], profile[PATH_MAX], application[PATH_MAX];
    uint32_t size = sizeof(executable);
    if (_NSGetExecutablePath(executable, &size) || !realpath(executable, resolved)) return 126;
    char *separator = strrchr(resolved, '/');
    if (!separator) return 126;
    *separator = '\0';
    int profile_length = snprintf(profile, sizeof(profile), "%s/../Resources/offline-deny-network.sb", resolved);
    int application_length = snprintf(application, sizeof(application), "%s/travel-gallery-offline-child", resolved);
    if (profile_length < 0 || (size_t)profile_length >= sizeof(profile)
        || application_length < 0 || (size_t)application_length >= sizeof(application))
        return 126;
    char *arguments[] = { "/usr/bin/sandbox-exec", "-f", profile, application, NULL };
    execv(arguments[0], arguments);
    perror("Offline QA launcher");
    return 126;
}
