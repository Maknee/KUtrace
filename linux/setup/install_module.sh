#!/bin/bash

cd module
make -j

sudo insmod kutrace_mod.ko tracemb=40 check=0
sudo rmmod kutrace_mod.ko

cd ../

cp postproc_changes/* postproc/

cd postproc
./build.sh


