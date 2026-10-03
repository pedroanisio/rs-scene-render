// Generate independent decoder fixtures with the official OpenVDB library.
// Build this development-only tool with OpenVDB 10+ headers/libraries; the
// renderer and Rust tests consume the checked-in files without this dependency.
#include <openvdb/openvdb.h>
#include <openvdb/io/File.h>
#include <filesystem>
#include <iostream>

int main(int argc, char** argv) {
    if (argc != 2) { std::cerr << "usage: openvdb-fixture OUTPUT_DIRECTORY\n"; return 2; }
    openvdb::initialize();
    const std::filesystem::path output(argv[1]);
    std::filesystem::create_directories(output);
    auto save = [&](const char* name, openvdb::GridPtrVec grids, uint32_t compression) {
        openvdb::io::File file((output / (std::string(name) + ".vdb")).string());
        file.setCompression(compression);
        file.write(grids);
    };
    auto density = openvdb::FloatGrid::create(0.0f);
    density->setName("density");
    density->setGridClass(openvdb::GRID_FOG_VOLUME);
    density->tree().setValue(openvdb::Coord(1,2,3), 1.25f);
    density->tree().setValue(openvdb::Coord(-1,-2,-3), 2.5f);
    density->tree().setValueOff(openvdb::Coord(4,5,6), 0.75f);
    save("density-none", {density}, openvdb::io::COMPRESS_NONE);
    save("density-zip", {density}, openvdb::io::COMPRESS_ZIP | openvdb::io::COMPRESS_ACTIVE_MASK);
    save("density-blosc", {density}, openvdb::io::COMPRESS_BLOSC | openvdb::io::COMPRESS_ACTIVE_MASK);
    density->setSaveFloatAsHalf(true);
    save("density-half", {density}, openvdb::io::COMPRESS_ZIP | openvdb::io::COMPRESS_ACTIVE_MASK);
    density->setSaveFloatAsHalf(false);

    auto background = openvdb::FloatGrid::create(0.5f);
    background->setName("density");
    background->tree().setValue(openvdb::Coord(1,2,3), 3.0f);
    background->tree().setValueOff(openvdb::Coord(4,5,6), -0.5f);
    openvdb::math::Mat4d matrix = openvdb::math::Mat4d::identity();
    matrix(0,0)=2.; matrix(1,1)=3.; matrix(2,2)=-4.; matrix(0,1)=0.25;
    matrix(3,0)=10.; matrix(3,1)=20.; matrix(3,2)=30.;
    background->setTransform(openvdb::math::Transform::createLinearTransform(matrix));
    save("affine-background", {background}, openvdb::io::COMPRESS_ZIP | openvdb::io::COMPRESS_ACTIVE_MASK);

    auto tiles = openvdb::FloatGrid::create(0.0f);
    tiles->setName("density");
    tiles->tree().addTile(1, openvdb::Coord(-8,0,0), 2.0f, true);
    tiles->tree().addTile(1, openvdb::Coord(8,0,0), 0.75f, false);
    tiles->tree().addTile(2, openvdb::Coord(128,0,0), 1.0f, true);
    save("tiles", {tiles}, openvdb::io::COMPRESS_BLOSC | openvdb::io::COMPRESS_ACTIVE_MASK);
    auto large = openvdb::FloatGrid::create(0.0f);
    large->setName("density");
    large->tree().addTile(3, openvdb::Coord(0,0,0), 1.0f, true);
    save("large-tile", {large}, openvdb::io::COMPRESS_NONE);

    auto velocity = openvdb::Vec3SGrid::create(openvdb::Vec3f(0.25f,0.5f,0.75f));
    velocity->setName("velocity");
    velocity->tree().setValue(openvdb::Coord(1,2,3), openvdb::Vec3f(4.f,-5.f,6.f));
    save("vector", {density,velocity}, openvdb::io::COMPRESS_BLOSC | openvdb::io::COMPRESS_ACTIVE_MASK);

    auto instance = density->copy();
    instance->setName("temperature");
    instance->setTransform(openvdb::math::Transform::createLinearTransform(2.0));
    save("instanced", {density,instance}, openvdb::io::COMPRESS_ZIP | openvdb::io::COMPRESS_ACTIVE_MASK);
    auto unsupported = openvdb::BoolGrid::create(false);
    unsupported->setName("mask");
    save("unsupported-bool", {unsupported}, openvdb::io::COMPRESS_NONE);
    auto frustum = density->deepCopy();
    frustum->setTransform(openvdb::math::Transform::createFrustumTransform(
        openvdb::BBoxd(openvdb::Vec3d(0),openvdb::Vec3d(10)), 0.5, 10.0));
    save("frustum", {frustum}, openvdb::io::COMPRESS_NONE);
    for (int frame = 0; frame < 2; ++frame) {
        auto smoke = openvdb::FloatGrid::create(frame == 0 ? 1.f : 3.f);
        auto heat = openvdb::FloatGrid::create(frame == 0 ? 2000.f : 8000.f);
        smoke->setName("density"); heat->setName("temperature");
        save(frame == 0 ? "render-0" : "render-1", {smoke, heat},
             openvdb::io::COMPRESS_BLOSC | openvdb::io::COMPRESS_ACTIVE_MASK);
    }
    auto dense = openvdb::FloatGrid::create(0.f);
    dense->setName("density");
    for (int x=0; x<8; ++x) for (int y=0; y<8; ++y) for (int z=0; z<8; ++z)
        dense->tree().setValue(openvdb::Coord(x,y,z), 0.25f+x+0.5f*y+0.125f*z);
    save("dense-zip", {dense}, openvdb::io::COMPRESS_ZIP | openvdb::io::COMPRESS_ACTIVE_MASK);
    save("dense-blosc", {dense}, openvdb::io::COMPRESS_BLOSC | openvdb::io::COMPRESS_ACTIVE_MASK);
    auto precise = openvdb::DoubleGrid::create(0.25);
    precise->setName("density");
    precise->tree().setValue(openvdb::Coord(-1,2,-3), 1.125);
    save("double", {precise}, openvdb::io::COMPRESS_ZIP | openvdb::io::COMPRESS_ACTIVE_MASK);
    auto vectorDouble = openvdb::Vec3DGrid::create(openvdb::Vec3d(0.25,0.5,0.75));
    vectorDouble->setName("velocity");
    vectorDouble->tree().setValue(openvdb::Coord(-1,2,-3), openvdb::Vec3d(4,-5,6));
    save("vector-double", {vectorDouble}, openvdb::io::COMPRESS_BLOSC | openvdb::io::COMPRESS_ACTIVE_MASK);
    std::cout << "Generated OpenVDB fixtures with library " << OPENVDB_LIBRARY_VERSION_STRING << "\n";
}
