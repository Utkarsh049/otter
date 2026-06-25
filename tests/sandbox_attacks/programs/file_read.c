#include <stdio.h>
int main() {
    FILE *f = fopen("/etc/passwd", "r");
    if (f == NULL) {
        printf("READ_FAILED\n");
    } else {
        printf("READ_SUCCESS\n");
        fclose(f);
    }
    return 0;
}
