#include <stdio.h>
#include <stdint.h>

static int64_t dist_sq(int64_t ax, int64_t ay, int64_t az,
                       int64_t bx, int64_t py, int64_t bz) {
    int64_t dx = ax - bx, dy = ay - py, dz = az - bz;
    return dx*dx + dy*dy + dz*dz;
}

int main(void) {
    int64_t x0=0,y0=0,z0=0,vx0=0,vy0=0,vz0=100,m0=1000;
    int64_t x1=1000,y1=0,z1=0,vx1=0,vy1=50,vz1=0,m1=10;
    int64_t x2=0,y2=1000,z2=0,vx2=-50,vy2=0,vz2=0,m2=10;
    int64_t x3=500,y3=500,z3=500,vx3=-20,vy3=20,vz3=-20,m3=5;
    int64_t x4=-500,y4=-500,z4=-500,vx4=30,vy4=-10,vz4=30,m4=5;

    for (int step = 0; step < 10000000; step++) {
        int64_t d = dist_sq(x0,y0,z0,x1,y1,z1) + 1;
        vx1 += (x0-x1)*m0/d;
        vy1 += (y0-y1)*m0/d;

        d = dist_sq(x0,y0,z0,x2,y2,z2) + 1;
        vx2 += (x0-x2)*m0/d;
        vy2 += (y0-y2)*m0/d;

        x0+=vx0; y0+=vy0; z0+=vz0;
        x1+=vx1; y1+=vy1; z1+=vz1;
        x2+=vx2; y2+=vy2; z2+=vz2;
        x3+=vx3; y3+=vy3; z3+=vz3;
        x4+=vx4; y4+=vy4; z4+=vz4;
    }
    printf("%ld\n", x0+x1+x2+x3+x4);
    return 0;
}
