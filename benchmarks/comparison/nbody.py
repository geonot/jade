def dist_sq(ax, ay, az, bx, py, bz):
    dx = ax - bx
    dy = ay - py
    dz = az - bz
    return dx*dx + dy*dy + dz*dz

x0,y0,z0,vx0,vy0,vz0,m0 = 0,0,0,0,0,100,1000
x1,y1,z1,vx1,vy1,vz1 = 1000,0,0,0,50,0
x2,y2,z2,vx2,vy2,vz2 = 0,1000,0,-50,0,0
x3,y3,z3,vx3,vy3,vz3 = 500,500,500,-20,20,-20
x4,y4,z4,vx4,vy4,vz4 = -500,-500,-500,30,-10,30

for _ in range(10000000):
    d = dist_sq(x0,y0,z0,x1,y1,z1) + 1
    vx1 += (x0-x1)*m0//d
    vy1 += (y0-y1)*m0//d

    d = dist_sq(x0,y0,z0,x2,y2,z2) + 1
    vx2 += (x0-x2)*m0//d
    vy2 += (y0-y2)*m0//d

    x0+=vx0; y0+=vy0; z0+=vz0
    x1+=vx1; y1+=vy1; z1+=vz1
    x2+=vx2; y2+=vy2; z2+=vz2
    x3+=vx3; y3+=vy3; z3+=vz3
    x4+=vx4; y4+=vy4; z4+=vz4

print(x0+x1+x2+x3+x4)
