import React, {useState, useEffect} from 'react';
import './App.css';
import {Version, version} from "./api";
import {CircularProgress} from "@material-ui/core";


const VersionComp: React.FC = () => {
    const [data, setData] = useState<Version | null>(null);
    useEffect(() => {
        if(data !=null){
            return;
        }
        const fetchData = async () => {
            const result = await version();
            setData(result);
        };
        fetchData();
    });

    return data == null ?
        <div>
            <CircularProgress />
        </div>
        :
        <div>
            <p>director: {data?.director}</p>
            <p>core: {data?.core}</p>
            <p>protocol: {data?.protocol}</p>
            <p>query_parser: {data?.query_parser}</p>
        </div>
};

export default VersionComp;
