#include <nanobind/nanobind.h>
#include <nanobind/stl/string.h>
#include <stdexcept>

#include "SteamSession.hpp"
#include "ManifestFileList.hpp"
#include "SteamError.hpp"

namespace nb = nanobind;

[[noreturn]] static void throw_steam_error(std::unique_ptr<SteamError> err) {
    std::string msg = err->message();
    throw std::runtime_error(msg);
}

struct PySteamSession {
    std::unique_ptr<SteamSession> inner;
};

struct PyManifestFileList {
    std::unique_ptr<ManifestFileList> inner;
};

NB_MODULE(steam_ffi_ext, m) {
    m.doc() = "Python bindings for the Steam depot downloader FFI";

    nb::class_<PyManifestFileList>(m, "ManifestFileList")
        .def("__len__", [](const PyManifestFileList& self) { return self.inner->len(); })
        .def("get_name", [](const PyManifestFileList& self, size_t i) { return self.inner->get_name(i); })
        .def("get_size", [](const PyManifestFileList& self, size_t i) { return self.inner->get_size(i); })
        .def("is_directory", [](const PyManifestFileList& self, size_t i) { return self.inner->is_directory(i); });

    nb::class_<PySteamSession>(m, "SteamSession")
        .def_static("connect_anonymous", []() -> PySteamSession {
            auto r = SteamSession::connect_anonymous();
            if (!r.is_ok()) throw_steam_error(*std::move(r).err());
            return PySteamSession{*std::move(r).ok()};
        })
        .def_static("connect_with_token",
            [](const std::string& username, const std::string& token) -> PySteamSession {
                auto r = SteamSession::connect_with_token(username, token);
                if (!r.is_ok()) throw_steam_error(*std::move(r).err());
                return PySteamSession{*std::move(r).ok()};
            },
            nb::arg("username"), nb::arg("token"))
        .def("list_depot_files",
            [](const PySteamSession& self, uint32_t app_id, uint32_t depot_id, const std::string& branch)
                -> PyManifestFileList {
                auto r = self.inner->list_depot_files(app_id, depot_id, branch);
                if (!r.is_ok()) throw_steam_error(*std::move(r).err());
                return PyManifestFileList{*std::move(r).ok()};
            },
            nb::arg("app_id"), nb::arg("depot_id"), nb::arg("branch") = "public");
}
