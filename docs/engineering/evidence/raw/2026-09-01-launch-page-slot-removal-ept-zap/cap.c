#include <stdio.h>
#include <fcntl.h>
#include <sys/ioctl.h>
#include <unistd.h>
#include <linux/kvm.h>
struct { int n; const char *name; } caps[] = {
  {231, "KVM_CAP_USER_MEMORY2"},
  {232, "KVM_CAP_MEMORY_FAULT_INFO"},
  {233, "KVM_CAP_MEMORY_ATTRIBUTES"},
  {234, "KVM_CAP_GUEST_MEMFD"},
  {235, "KVM_CAP_VM_TYPES"},
  {236, "KVM_CAP_PRE_FAULT_MEMORY (upstream number)"},
  {237, "237 (unknown here)"},
  {0, 0}
};
int main(void){
  int fd = open("/dev/kvm", O_RDWR);
  if (fd < 0) { perror("open /dev/kvm"); return 1; }
  printf("KVM_GET_API_VERSION = %d\n", ioctl(fd, KVM_GET_API_VERSION, 0));
  for (int i = 0; caps[i].name; i++)
    printf("KVM_CHECK_EXTENSION(%3d) = %2d   %s\n", caps[i].n,
           ioctl(fd, KVM_CHECK_EXTENSION, caps[i].n), caps[i].name);
  return 0;
}
