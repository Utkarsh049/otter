#include <stdio.h>
int main() {
    FILE *f = fopen("large_file.txt", "w");
    if (f == NULL) return 1;
    // Write 5MB
    for (int i = 0; i < 5 * 1024 * 1024; i++) {
        if (fputc('A', f) == EOF) {
            break;
        }
    }
    fclose(f);
    return 0;
}
